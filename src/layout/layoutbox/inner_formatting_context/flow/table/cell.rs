//! Single-cell layout (`<td>` / `<th>`) — invoked by [`super::table::layout_table`]
//! once the column width has been computed.

use crate::{
    html::box_edges::BoxEdges,
    layout::{
        BoxConstraint, Cursor, FontContext, LayoutBox, PaintStyle, Rect,
        inner_formatting_context::flow::layout_children, layoutbox::collect_runs,
    },
    tdom::node::{NodeArena, NodeId, effective_font_size_cells},
    tui::graph_api::block_style,
};

/// Information about a single cell inside a row.
#[derive(Clone, Copy)]
pub struct CellInfo {
    pub node_id: NodeId,
    /// `true` for `<th>` cells; the font-style / alignment defaults are
    /// surfaced for callers that want to apply them.
    pub is_header: bool,
}

/// Build a single cell's [`LayoutBox`] at the given `rect`.
pub fn layout_cell(
    arena: &NodeArena,
    cell_id: NodeId,
    rect: Rect,
    _is_header: bool,
    font_ctx: FontContext,
) -> LayoutBox {
    let node = &arena[cell_id];

    let own_font_size_cells = effective_font_size_cells(
        arena,
        cell_id,
        font_ctx.font_size_cells,
        font_ctx.root_font_size_cells,
    );
    let child_font_ctx = font_ctx.child(own_font_size_cells);

    let cell_edges: BoxEdges = node.css_attr.get_box_edges(
        rect.width,
        own_font_size_cells,
        font_ctx.root_font_size_cells,
    );

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let body_runs = collect_runs(arena, node, block_fg, block_bg, block_font_style);

    let fill_bg = block_bg.is_some();

    let inner_content_x = rect.x + cell_edges.border.left.thickness() + cell_edges.padding.left;
    let inner_content_y = rect.y + cell_edges.border.top.thickness() + cell_edges.padding.top;
    let inner_content_width = rect
        .width
        .saturating_sub(cell_edges.inner_horizontal())
        .max(1);
    let mut child_cursor = Cursor { y: inner_content_y };
    let child_containing = BoxConstraint {
        x: inner_content_x,
        width: inner_content_width,
    };
    let children = layout_children(
        arena,
        &node.children,
        child_containing,
        &mut child_cursor,
        child_font_ctx,
    );

    let paint_style = PaintStyle {
        box_edges: cell_edges,
        runs: if body_runs.is_empty() { None } else { Some(body_runs) },
        font_size_cells: own_font_size_cells,
        fill_bg,
        spacing_after: 0,
        chrome: false,
    };

    LayoutBox::new_normal(
        cell_id,
        rect,
        paint_style,
        node.css_attr.outer_display(),
        children,
    )
}
