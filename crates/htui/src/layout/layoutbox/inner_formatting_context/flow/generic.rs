use crossterm::style::Color;

use crate::{
    layout::{
        BoxConstraint, Cursor, InlineRun, LayoutBox, LayoutContext,
        inner_formatting_context::flow::{LayoutBlockOptions, layout_block},
    },
    tdom::node::{Node, NodeArena, NodeId, NodeType},
    tui::graph_api::block_style,
};

/// Build a plain block box for block-level elements without specialised
/// rendering rules.
///
/// This is the final block fallback in the registry. Specialised handlers for
/// headings, lists, tables, images, chrome, and the document root run first;
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
        (ratio, None)
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
