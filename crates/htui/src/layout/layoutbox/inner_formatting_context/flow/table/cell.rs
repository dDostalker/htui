//! Single-cell layout (`<td>` / `<th>`) — invoked by [`super::table::layout_table`]
//! once the column width has been computed.

use crate::{
    html::box_edges::BoxEdges,
    layout::{
        BoxConstraint, Cursor, LayoutBox, LayoutContext, PaintStyle, Rect,
        inner_formatting_context::flow::layout_children,
        layoutbox::{collect_runs, wrap_runs},
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

/// Number of rows a cell needs at the given column width: the wrapped
/// inline-text height (each visual line takes `font_size_cells` rows) plus
/// the cell's own vertical border / padding. Row heights drive the table
/// geometry; without this, a cell whose text wraps (common when the terminal
/// is narrow enough that the column shrinks) would paint its wrapped lines
/// over the table's bottom border.
pub fn cell_required_height(
    arena: &NodeArena,
    cell_id: NodeId,
    col_width: usize,
    context: LayoutContext,
) -> usize {
    let node = &arena[cell_id];
    let own_font_size_cells =
        effective_font_size_cells(arena, cell_id, context.font, context.viewport);
    let child_context = context.child(own_font_size_cells);
    let cell_edges: BoxEdges =
        node.css_attr
            .get_box_edges(col_width, child_context.font, child_context.viewport);
    let content_width = col_width
        .saturating_sub(cell_edges.inner_horizontal())
        .max(1);

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let body_runs = collect_runs(arena, cell_id, node, block_fg, block_bg, block_font_style);
    let line_count = if body_runs.is_empty() {
        1
    } else {
        wrap_runs(
            &body_runs,
            content_width,
            own_font_size_cells,
            node.css_attr.get_white_space(&node.node_type),
            node.css_attr.get_word_break(),
            node.css_attr.get_overflow_wrap(),
        )
        .len()
        .max(1)
    };

    line_count
        .saturating_mul(own_font_size_cells.max(1))
        .saturating_add(cell_edges.border.top.thickness())
        .saturating_add(cell_edges.padding.top)
        .saturating_add(cell_edges.border.bottom.thickness())
        .saturating_add(cell_edges.padding.bottom)
        .max(1)
}

/// Build a single cell's [`LayoutBox`] at the given `rect`.
pub fn layout_cell(
    arena: &NodeArena,
    cell_id: NodeId,
    rect: Rect,
    _is_header: bool,
    context: LayoutContext,
) -> LayoutBox {
    let node = &arena[cell_id];

    let own_font_size_cells =
        effective_font_size_cells(arena, cell_id, context.font, context.viewport);
    let child_context = context.child(own_font_size_cells);

    let cell_edges: BoxEdges =
        node.css_attr
            .get_box_edges(rect.width, child_context.font, child_context.viewport);

    let block_fg = node.css_attr.get_color();
    let block_bg = node.css_attr.get_bg_color();
    let block_font_style = block_style(&node.css_attr);
    let body_runs = collect_runs(arena, cell_id, node, block_fg, block_bg, block_font_style);

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
        child_context,
    );

    // Wrap the cell's own inline text so long cell content wraps
    // instead of overflowing. A `<td>` only owns its own text, so we
    // wrap the body runs to the cell's content-box width.
    let cell_content_width = inner_content_width.max(1);
    let wrapped_lines = if body_runs.is_empty() {
        Vec::new()
    } else {
        wrap_runs(
            &body_runs,
            cell_content_width,
            own_font_size_cells,
            node.css_attr.get_white_space(&node.node_type),
            node.css_attr.get_word_break(),
            node.css_attr.get_overflow_wrap(),
        )
    };

    let paint_style = PaintStyle {
        box_edges: cell_edges,
        runs: if wrapped_lines.is_empty() || wrapped_lines.iter().all(|l| l.is_empty()) {
            None
        } else {
            Some(wrapped_lines)
        },
        font_size_cells: own_font_size_cells,
        line_height_cells: own_font_size_cells.max(1),
        border_radius: crate::html::css_parse::CornerRadii::default(),
        fill_bg,
        spacing_after: 0,
    };

    LayoutBox::new_normal(
        cell_id,
        rect,
        paint_style,
        node.css_attr.outer_display(&node.node_type),
        children,
    )
}
