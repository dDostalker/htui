//! Shared body of every block-level layout function.
//!
//! [`layout_block`] is the single seam where every per-element handler
//! converges. It walks the same pipeline (resolve width → compute edges →
//! allocate rect → render children → reserve next row) and the element
//! only contributes its options. Keeping the pipeline in one place means
//! adding a new block element only requires a few lines that pick the
//! right [`LayoutBlockOptions`].

use crate::{
    html::box_edges::BoxEdges,
    layout::{
        BoxConstraint, Cursor, LayoutBox, LayoutContext, PaintStyle, Rect,
        inner_formatting_context::flow::layout_children,
        layoutbox::collect_runs,
    },
    tdom::node::{Node, NodeArena, NodeId, effective_font_size_cells},
    tui::graph_api::block_style,
};

use super::geometry::{
    OuterGeometry, block_left, build_border_rect, compute_outer_geometry, reserve_border_box,
    resolve_block_width,
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

    let edges: BoxEdges = node
        .css_attr
        .get_box_edges(containing.width, child_context.font, child_context.viewport);

    let (geom, preallocated_height) = match pre_allocated_rect {
        Some(rect) => {
            let inner_h = edges.inner_horizontal();
            let content_constraint = BoxConstraint {
                x: rect.x
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
            let available = containing
                .width
                .saturating_sub(opts.indent)
                .saturating_sub(edges.margin.horizontal());
            let block_width = resolve_block_width(
                node.css_attr
                    .get_width(available, child_context.font, child_context.viewport),
                available,
            )
            .max(1);
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
    let block_font_style = block_style(&node.css_attr);
    let body_runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);
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
    let mut child_cursor = Cursor {
        y: geom.outer_y + edges.border.top.thickness() + edges.padding.top,
    };
    let block_children = node.children.clone();
    let children = layout_children(
        arena,
        &block_children,
        child_containing,
        &mut child_cursor,
        child_context,
    );

    let content_height = child_cursor
        .y
        .saturating_sub(geom.outer_y + edges.border.top.thickness() + edges.padding.top);
    let outer_height = match preallocated_height {
        Some(height) => height.max(1),
        None => reserve_border_box(
            cursor,
            geom.outer_y,
            edges,
            content_height,
            own_font_size_cells.max(1),
        ),
    };

    let mut runs = body_runs;
    if let Some(prefix) = opts.prefix {
        runs.insert(0, prefix);
    }

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
        runs: if runs.is_empty() { None } else { Some(runs) },
        font_size_cells: own_font_size_cells,
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
