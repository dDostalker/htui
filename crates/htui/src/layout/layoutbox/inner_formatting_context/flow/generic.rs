use std::sync::Arc;

use crossterm::style::Color;

use crate::{
    graphics::block_style,
    layout::{
        BoxConstraint, Cursor, InlineRun, LayoutBox, LayoutContext,
        inner_formatting_context::flow::{LayoutBlockOptions, layout_block},
        layoutbox::{
            generation::{GeneratedBoxKind, generate_node},
            q_quote_pair,
        },
    },
    tdom::node::{Node, NodeArena, NodeId, NodeType},
};

/// Build a plain block box for block-level elements without specialised
/// rendering rules.
///
/// This is the final block fallback in the registry. Specialised handlers for
/// headings, lists, tables, images, and the document root run first;
/// this handler keeps ordinary structural elements usable in both normal block
/// flow and as flex items.
pub fn layout_generic(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if !is_generic_block(&node.node_type) {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions::default(),
        context,
        pre_allocated_rect,
    ))
}

fn is_generic_block(node_type: &NodeType) -> bool {
    matches!(
        node_type,
        NodeType::Nav
            | NodeType::Main
            | NodeType::Section
            | NodeType::Article
            | NodeType::Figure
            | NodeType::Figcaption
            | NodeType::Details
            | NodeType::Input
            | NodeType::Form
            | NodeType::Select
            | NodeType::Pre
    )
}

/// Blockify an inline element that a flex container has promoted to a flex
/// item.
///
/// CSS Flexbox blockifies every in-flow child: an inline `<span>` / `<em>` /
/// `<label>` … lays out as a block box containing its own inline runs, not as
/// a fragment of the flex container's text. This handler only claims inline
/// normal boxes once a parent has pre-allocated their rectangle
/// (`pre_allocated_rect.is_some()`). In ordinary block flow inline elements
/// never reach the geometry dispatcher (their runs are folded into the parent
/// by `collect_runs`), so returning `None` there keeps that path intact.
pub fn layout_inline_blockify(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let generated = generate_node(arena, node_id)?;
    if generated.kind != GeneratedBoxKind::Normal || !generated.outer_display.is_inline() {
        return None;
    }
    let pre_allocated_rect = pre_allocated_rect?;

    let mut node = arena[node_id].clone();
    apply_inline_ua_defaults(&mut node);

    // `<q>` carries generated quote marks around its content. The shared block
    // pipeline renders them as a prepended open quote and an appended close
    // quote, using the element's own fg / bg / font style.
    let opts = if node.node_type == NodeType::Q {
        let (open, close) = q_quote_pair(arena, node_id);
        let fg = node.css_attr.get_color();
        let bg = node.css_attr.get_bg_color();
        let style = block_style(&node.css_attr);
        LayoutBlockOptions {
            prefix: Some(InlineRun::new(open, style, fg, bg)),
            suffix: Some(InlineRun::new(close, style, fg, bg)),
            ..Default::default()
        }
    } else {
        LayoutBlockOptions::default()
    };

    Some(layout_block(
        arena,
        &node,
        node_id,
        containing,
        cursor,
        opts,
        context,
        Some(pre_allocated_rect),
    ))
}

/// Inject the browser-style inline UA defaults into a blockified inline
/// element's `css_attr`. Each default only lands when the cascade has not
/// already set the matching property, so an author rule or inherited value
/// always wins.
fn apply_inline_ua_defaults(node: &mut Node) {
    for &(prop, value) in node.node_type.inline_ua_declarations() {
        if !node.css_attr.is_set(prop) {
            node.css_attr.0.insert(Arc::from(prop), Arc::from(value));
        }
    }
    // `<abbr>`'s underline is conditional on a `title` attribute.
    if node.node_type == NodeType::Abbr
        && node.attributes.get("title").is_some()
        && !node.css_attr.is_set("text-decoration")
    {
        node.css_attr
            .0
            .insert(Arc::from("text-decoration"), Arc::from("underline"));
    }
}

/// Build a [`LayoutBox`] for a `<summary>` element — the disclosure toggle of
/// a `<details>` widget.
///
/// The browser UA stylesheet renders a summary as a list item whose
/// `::marker` is a disclosure triangle (`disclosure-closed` / `disclosure-open`).
/// This engine synthesises the same marker as an inline prefix: `▸ ` while the
/// owning `<details>` is closed, `▾ ` once it is open. A `<summary>` outside a
/// `<details>` has no marker, exactly like a browser.
pub fn layout_summary(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Summary {
        return None;
    }
    let prefix = summary_marker(arena, node_id).map(|marker| {
        let fg = node.css_attr.get_color();
        let bg = node.css_attr.get_bg_color();
        let style = block_style(&node.css_attr);
        InlineRun::new(marker, style, fg, bg)
    });
    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions {
            prefix,
            ..Default::default()
        },
        context,
        pre_allocated_rect,
    ))
}

/// The disclosure marker for a `<summary>`: `▸ ` (closed) / `▾ ` (open) when
/// the element's parent is a `<details>`, `None` otherwise.
fn summary_marker(arena: &NodeArena, node_id: NodeId) -> Option<String> {
    let parent_id = arena[node_id].parent?;
    if arena[parent_id].node_type != NodeType::Details {
        return None;
    }
    let open = arena[parent_id].attributes.contains_key("open");
    Some(if open { "▾ " } else { "▸ " }.to_string())
}

/// Build a [`LayoutBox`] for an `<progress>` / `<meter>` element — a
/// terminal bar (`█` filled / `░` empty) spanning the content-box width.
///
/// The fill ratio comes from `value` / `max` (`<progress>` defaults `max` to
/// 1; `<meter>` resolves `min` / `max`, defaulting to 0 / 1). `<meter>`
/// colours the filled cells by the value's position against `low` / `high` /
/// `optimum`: red outside `[low, high]`, green between `optimum` and the
/// range midpoint, amber on the far side — the browser's gauge semantics.
pub fn layout_progress(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if !matches!(node.node_type, NodeType::Progress | NodeType::Meter) {
        return None;
    }
    let (ratio, color) = progress_ratio(node);
    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions {
            bar_fill: Some(ratio),
            bar_color: color,
            ..Default::default()
        },
        context,
        pre_allocated_rect,
    ))
}

/// Resolve the `(fill ratio, bar color)` of a `<progress>` / `<meter>` from
/// its `value` / `min` / `max` / `low` / `high` / `optimum` attributes.
fn progress_ratio(node: &Node) -> (f32, Option<Color>) {
    let attr = |name: &str, default: f32| {
        node.attributes
            .get(name)
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(default)
    };
    let value = attr("value", 0.0);
    if node.node_type == NodeType::Meter {
        let min = attr("min", 0.0);
        let max = attr("max", 1.0);
        let ratio = if max > min {
            ((value - min) / (max - min)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let low = attr("low", min);
        let high = attr("high", max);
        let optimum = attr("optimum", (min + max) / 2.0);
        // Browser gauge tiers: red outside the good range, green between
        // `optimum` and the range midpoint, amber on the far side.
        let mid = (low + high) / 2.0;
        let color = if value < low || value > high {
            Color::Red
        } else if (value >= optimum) == (mid >= optimum) {
            Color::Green
        } else {
            Color::Yellow
        };
        (ratio, Some(color))
    } else {
        let max = attr("max", 1.0);
        let ratio = if max > 0.0 {
            (value / max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        // `accent-color` themes the fill; `None` means "use element fg".
        let color = node.css_attr.get_accent_color();
        (ratio, color)
    }
}

/// Build a [`LayoutBox`] for an `<hr>` element.
///
/// `<hr>` renders a thematic break as a horizontal rule: a single row of
/// `─` glyphs spanning the content-box width (the browser equivalent of
/// `display: block` + a 1px top border). The rule run is synthesised at
/// layout time from the element's own fg / bg / font style, and one blank
/// row is reserved below so the rule does not abut the next block.
pub fn layout_hr(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<crate::layout::Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Hr {
        return None;
    }
    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions {
            hr_rule: true,
            spacing_after: 1,
            ..Default::default()
        },
        context,
        pre_allocated_rect,
    ))
}
