//! Shared body of every block-level layout function.
//!
//! [`layout_block`] is the single seam where every per-element handler
//! converges. It walks the same pipeline (resolve width → compute edges →
//! allocate rect → render children → reserve next row) and the element
//! only contributes its options. Keeping the pipeline in one place means
//! adding a new block element only requires a few lines that pick the
//! right [`LayoutBlockOptions`].

use crate::{
    html::box_edges::{Border, BorderSides, BoxEdges, EdgeSizes},
    html::css_parse::BoxSizing,
    layout::{
        BoxConstraint, Cursor, InlineRun, LayoutBox, LayoutContext, PaintStyle, Rect,
        inner_formatting_context::flow::layout_children,
        layoutbox::{collect_runs, visible_children, wrap_runs},
    },
    tdom::node::{Node, NodeArena, NodeId, effective_font_size_cells},
    tui::graph_api::{FontStyle, block_style},
};

use crate::html::css_parse::resolve_line_height_cells;

use super::geometry::{
    OuterGeometry, block_left, build_border_rect, compute_outer_geometry, constrained_block_width,
    reserve_border_box,
};
use super::options::LayoutBlockOptions;

/// Shared body of every block-level layout function.
///
/// Resolves the CSS `width`, padding / border / margin, and the inline
/// `runs`, then either reserves a border-box in normal block flow or honors
/// the border-box supplied by a parent formatting context. The element-specific
/// differences are passed in via `opts` so the boilerplate (collect_runs,
/// child layout, and box construction) lives in one place.
///
/// `context` carries the parent's effective font-size, the root font-size,
/// and the terminal viewport so `em` / `rem` / `vw` / `vh` lengths inside the
/// element resolve against the active layout context. The element's own
/// font-size is computed via [`effective_font_size_cells`] and forwarded to
/// nested children via `context.child(...)` so the chain remains consistent.
///
/// Returns the assembled `LayoutBox`. The caller is still responsible for
/// checking that `node.node_type` matches the element it claims to lay out
/// (so this helper stays type-agnostic).
pub fn layout_block(
    arena: &NodeArena,
    node: &Node,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    opts: LayoutBlockOptions,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> LayoutBox {
    let own_font_size_cells =
        effective_font_size_cells(arena, node_id, context.font, context.viewport);
    let child_context = context.child(own_font_size_cells);

    let edges: BoxEdges =
        node.css_attr
            .get_box_edges(containing.width, child_context.font, child_context.viewport);
    // Layer the browser UA default margins (`opts.ua_margins`) on top of
    // the author CSS edges. Each side keeps its UA value only while neither
    // the `margin` shorthand nor that side's longhand is declared by the
    // author — the same precedence an author stylesheet has over the UA
    // stylesheet in the browser cascade.
    let edges = merge_ua_margins(node, edges, opts.ua_margins);
    let edges = merge_ua_border(node, edges, opts.ua_border);
    let sizing = node.css_attr.get_box_sizing();
    let line_height_cells = resolve_line_height_cells(
        node.css_attr.get_line_height(),
        own_font_size_cells,
        child_context.font,
        child_context.viewport,
    );
    // The available width after margins / indent — shared by the width
    // resolution and the min/max-height clamp below.
    let available = containing
        .width
        .saturating_sub(opts.indent)
        .saturating_sub(edges.margin.horizontal());

    let (geom, preallocated_height) = match pre_allocated_rect {
        Some(rect) => {
            let inner_h = edges.inner_horizontal();
            let content_constraint = BoxConstraint {
                x: rect
                    .x
                    .saturating_add(edges.border.left.thickness())
                    .saturating_add(edges.padding.left),
                width: rect.width.saturating_sub(inner_h).max(1),
            };
            (
                OuterGeometry {
                    outer_x: rect.x,
                    outer_y: rect.y,
                    outer_width: rect.width,
                    content_constraint,
                },
                Some(rect.height),
            )
        }
        None => {
            cursor.skip(edges.margin.top);
            let outer_y_top = cursor.y;
            let block_width = constrained_block_width(
                &node.css_attr,
                available,
                edges,
                child_context.font,
                child_context.viewport,
            );
            let x = block_left(containing) + opts.indent;
            let geom = compute_outer_geometry(
                BoxConstraint {
                    x,
                    width: block_width,
                },
                block_width,
                edges,
                outer_y_top,
            );
            (geom, None)
        }
    };
    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let mut block_font_style = block_style(&node.css_attr);
    // User-agent default: `<address>` renders italic. Anything the author
    // (or an ancestor rule) declared for `font-style` takes precedence —
    // `address { font-style: normal }` is honoured (same cascade-precedence
    // rule the `<em>` UA default follows).
    if opts.ua_italic && !node.css_attr.is_set("font-style") {
        block_font_style |= FontStyle::ITALIC;
    }
    // `<hr>` has no inline content of its own; skip the (empty) walk.
    let body_runs = if opts.hr_rule {
        Vec::new()
    } else {
        collect_runs(arena, node_id, node, block_fg, block_bg, block_font_style)
    };
    let fill_bg = block_bg.is_some() || opts.fill_bg_default;

    let child_containing = match opts.child_containing_shift {
        Some((x_off, w_delta)) => BoxConstraint {
            x: geom.content_constraint.x.saturating_add(x_off),
            width: if w_delta >= 0 {
                geom.content_constraint
                    .width
                    .saturating_add(w_delta as usize)
                    .max(1)
            } else {
                geom.content_constraint
                    .width
                    .saturating_sub((-w_delta) as usize)
                    .max(1)
            },
        },
        None => geom.content_constraint,
    };
    // The marker / hash prefix belongs to the first visual line. Prepend
    // it to the flat run list so the line-breaker treats it as part of
    // the first "word" group and renders it on the top row only.
    let mut all_runs = if opts.hr_rule {
        // `<hr>` paints a horizontal rule: one `─` glyph per content cell,
        // synthesised at layout time so it always spans the available width.
        let width = geom.content_constraint.width.max(1);
        vec![InlineRun::new(
            "─".repeat(width),
            block_font_style,
            block_fg,
            block_bg,
        )]
    } else if let Some(ratio) = opts.bar_fill {
        // `<progress>` / `<meter>` paint a terminal bar: `█` for the filled
        // part, `░` for the rest, spanning the content-box width. The filled
        // cells carry `bar_color` (or the element fg); the empty cells keep
        // the element fg so the whole run reads as one widget.
        let width = geom.content_constraint.width.max(1);
        let filled = ((width as f32) * ratio.clamp(0.0, 1.0)).round() as usize;
        let mut run_text = "█".repeat(filled.min(width));
        run_text.push_str(&"░".repeat(width.saturating_sub(filled)));
        vec![InlineRun::new(
            run_text,
            block_font_style,
            opts.bar_color.or(block_fg),
            block_bg,
        )]
    } else {
        body_runs
    };
    if let Some(prefix) = opts.prefix.clone() {
        all_runs.insert(0, prefix);
    }

    // Wrap the inline content to the content-box width. The wrapped
    // line count drives the rect height; the per-line run list is
    // what the paint phase draws.
    let wrap_width = geom.content_constraint.width.max(1);
    let wrapped_lines = if all_runs.is_empty() {
        Vec::new()
    } else {
        wrap_runs(
            &all_runs,
            wrap_width,
            own_font_size_cells,
            node.css_attr.get_white_space(&node.node_type),
            node.css_attr.get_word_break(),
            node.css_attr.get_overflow_wrap(),
        )
    };
    let inline_line_count = wrapped_lines.len();
    let inline_rows = inline_line_count.saturating_mul(line_height_cells.max(1));

    // Block children begin BELOW the box's own inline text: in CSS the
    // inline content forms its own line(s) first and block-level children
    // start on a fresh line underneath. Starting the child cursor at the
    // content top would paint the first child's row over the parent's own
    // text (e.g. `<li>Nested list<ul>…</ul></li>` and
    // `<div>text<p>child</p></div>`).
    let content_top = geom.outer_y + edges.border.top.thickness() + edges.padding.top;
    let mut child_cursor = Cursor {
        y: content_top.saturating_add(inline_rows),
    };
    let block_children = visible_children(arena, node_id);
    let children = layout_children(
        arena,
        &block_children,
        child_containing,
        &mut child_cursor,
        child_context,
    );

    let children_rows = child_cursor
        .y
        .saturating_sub(content_top.saturating_add(inline_rows));

    // The content area stacks the wrapped inline lines and the block
    // children (they no longer share rows), so the border-box height
    // accommodates both.
    let content_height = children_rows.saturating_add(inline_rows);

    let outer_height = match preallocated_height {
        Some(height) => {
            // A pre-allocated rect (flex item placement) fixes the item's
            // border-box height, but it must never clip the item's own
            // content: the measured intrinsic size can under-count the
            // real layout (e.g. `<hr>` reserves a trailing row, wide runs
            // wrap to more lines than the max-content measure assumed).
            // Grow the rect to fit the content instead of letting the last
            // row bleed into the border.
            let content_border = content_height
                .max(line_height_cells.max(1))
                .saturating_add(edges.padding.top)
                .saturating_add(edges.padding.bottom)
                .saturating_add(edges.border.top.thickness())
                .saturating_add(edges.border.bottom.thickness());
            height.max(content_border)
        }
        None => {
            let mut height = reserve_border_box(
                cursor,
                geom.outer_y,
                edges,
                content_height,
                line_height_cells.max(1),
            );
            // `min-height` / `max-height` clamp the border-box height
            // (converted per `box-sizing`). `min-height` can push the box
            // past its content; `max-height` caps it (content is clipped by
            // the paint phase).
            let inner_v = edges.padding.top
                + edges.padding.bottom
                + edges.border.top.thickness()
                + edges.border.bottom.thickness();
            let to_border = |h: usize| match sizing {
                BoxSizing::BorderBox => h,
                BoxSizing::ContentBox => h.saturating_add(inner_v),
            };
            if let Some(m) = node
                .css_attr
                .get_min_height(available, child_context.font, child_context.viewport)
                .map(to_border)
            {
                height = height.max(m);
            }
            if let Some(m) = node
                .css_attr
                .get_max_height(available, child_context.font, child_context.viewport)
                .map(to_border)
            {
                height = height.min(m).max(1);
            }
            // `reserve_border_box` already advanced the cursor using the
            // pre-clamp height; re-position it so the next sibling starts
            // below the clamped border-box (otherwise the parent's content
            // area would stay short and the paint phase would clip this
            // box's extra rows).
            cursor.y = geom
                .outer_y
                .saturating_add(height)
                .saturating_add(edges.margin.bottom);
            height
        }
    };

    // Spacing belongs to block flow. A flex formatting context has already
    // allocated the item's border-box, so adding block-flow spacing here
    // would move the item outside the rectangle chosen by flex.
    let rect_height = if preallocated_height.is_some() {
        outer_height
    } else {
        outer_height + opts.spacing_after
    };
    if preallocated_height.is_none() && opts.spacing_after > 0 {
        cursor.y = cursor.y.saturating_add(opts.spacing_after);
    }

    let effective_spacing_after = if preallocated_height.is_some() {
        0
    } else {
        opts.spacing_after
    };
    let effective_chrome = opts.chrome || opts.fill_bg_default;

    let paint_style = PaintStyle {
        box_edges: edges,
        runs: if wrapped_lines.is_empty() || wrapped_lines.iter().all(|l| l.is_empty()) {
            None
        } else {
            Some(wrapped_lines)
        },
        font_size_cells: own_font_size_cells,
        line_height_cells,
        border_radius: node
            .css_attr
            .get_border_radius(containing.width, child_context.font, child_context.viewport),
        fill_bg,
        spacing_after: effective_spacing_after,
        chrome: effective_chrome,
    };

    LayoutBox::new_normal(
        node_id,
        build_border_rect(geom, rect_height),
        paint_style,
        node.css_attr.outer_display(&node.node_type),
        children,
    )
}

/// Layer the browser UA default border (`opts.ua_border`) onto the resolved
/// CSS edges, mirroring the cascade where the UA stylesheet loses to any
/// author declaration — but at component granularity, like a browser:
/// `dialog { border-width: 2px }` keeps the UA `solid` style / default color
/// while only the width comes from the author.
///
/// Each side keeps a UA component (width / style / color) only while neither
/// the `border` shorthand, that side's `border-{side}` shorthand, nor the
/// universal `border-{component}` longhand is declared by the author.
fn merge_ua_border(node: &Node, edges: BoxEdges, ua_border: Option<BorderSides>) -> BoxEdges {
    let Some(ua) = ua_border else {
        return edges;
    };
    let css = &node.css_attr;
    let authored = |side: &str, component: &str| -> bool {
        css.is_set("border")
            || css.is_set(&format!("border-{side}"))
            || css.is_set(&format!("border-{component}"))
    };
    let merge = |side: &str, author: Border, ua_side: Border| Border {
        width: if authored(side, "width") {
            author.width
        } else {
            ua_side.width
        },
        style: if authored(side, "style") {
            author.style
        } else {
            ua_side.style
        },
        color: if authored(side, "color") {
            author.color
        } else {
            ua_side.color
        },
    };
    BoxEdges {
        border: BorderSides {
            top: merge("top", edges.border.top, ua.top),
            right: merge("right", edges.border.right, ua.right),
            bottom: merge("bottom", edges.border.bottom, ua.bottom),
            left: merge("left", edges.border.left, ua.left),
        },
        ..edges
    }
}

/// Layer the browser UA default margins (`opts.ua_margins`) onto the
/// resolved CSS edges.
///
/// Each side keeps its UA value only while neither the `margin` shorthand
/// nor that side's longhand is declared by the author — the same precedence
/// an author stylesheet has over the UA stylesheet in the browser cascade.
fn merge_ua_margins(node: &Node, mut edges: BoxEdges, ua_margins: Option<EdgeSizes>) -> BoxEdges {
    let Some(ua) = ua_margins else {
        return edges;
    };
    if !node.css_attr.is_set("margin") && !node.css_attr.is_set("margin-top") {
        edges.margin.top = ua.top;
    }
    if !node.css_attr.is_set("margin") && !node.css_attr.is_set("margin-right") {
        edges.margin.right = ua.right;
    }
    if !node.css_attr.is_set("margin") && !node.css_attr.is_set("margin-bottom") {
        edges.margin.bottom = ua.bottom;
    }
    if !node.css_attr.is_set("margin") && !node.css_attr.is_set("margin-left") {
        edges.margin.left = ua.left;
    }
    edges
}
