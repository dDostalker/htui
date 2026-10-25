//! Layout the `<table>` element itself.

use crate::{
    layout::{BoxConstraint, Cursor, LayoutBox, LayoutContext, PaintStyle, Rect},
    tdom::node::{NodeArena, NodeId, NodeType, effective_font_size_cells},
};

use super::cell::{CellInfo, cell_required_height, layout_cell};
use super::measure::{collect_cells, collect_rows, compute_column_widths};

/// Build a [`LayoutBox`] for a `<table>` element.
pub fn layout_table(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    context: LayoutContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Table {
        // `display: table` on an arbitrary element is now allowed (the
        // generation phase no longer special-cases `<table>`); we only
        // log a debug hint so the table algorithm's expectations on its
        // row / cell children remain visible.
        log::debug!(
            "layout_table reached for non-table node {:?} (allowed by display: table); \
             children must be <tr> / <td>-shaped to render",
            node.node_type
        );
    }

    let own_font_size_cells =
        effective_font_size_cells(arena, node_id, context.font, context.viewport);
    let child_context = context.child(own_font_size_cells);

    let edges =
        node.css_attr
            .get_box_edges(containing.width, child_context.font, child_context.viewport);

    let available = (containing.width as i64 - edges.margin.horizontal() as i64).max(0) as usize;
    let (x, block_width, outer_y_top, preallocated_height) = match pre_allocated_rect {
        Some(rect) => (rect.x, rect.width.max(1), rect.y, Some(rect.height.max(1))),
        None => {
            let block_width = node
                .css_attr
                .get_width(available, child_context.font, child_context.viewport)
                .unwrap_or(available)
                .min(available)
                .max(1);
            // Collapse the top margin with the previous in-flow sibling's
            // bottom margin (CSS vertical margin collapsing: the larger wins).
            cursor.collapse_and_skip(edges.margin.top);
            (
                (containing.x as i64 + edges.margin.left as i64).max(0) as usize,
                block_width,
                cursor.y,
                None,
            )
        }
    };

    let rows = collect_rows(arena, node_id);
    let row_cells: Vec<Vec<CellInfo>> = rows
        .iter()
        .map(|&row_id| collect_cells(arena, row_id))
        .collect();

    let inner_width = block_width.saturating_sub(edges.inner_horizontal()).max(1);
    let column_widths = compute_column_widths(arena, &row_cells, inner_width);

    let total_columns_width: usize = column_widths.iter().sum();
    let total_table_width = total_columns_width
        .saturating_add(edges.border.horizontal_thickness())
        .saturating_add(edges.padding.horizontal());

    // Each row is as tall as its tallest cell. Column widths are already
    // clamped to the available width, so at narrow terminal widths a
    // cell's text wraps to several lines and the row must grow to keep
    // those lines above the table's bottom border.
    let row_heights: Vec<usize> = row_cells
        .iter()
        .map(|row| {
            row.iter()
                .enumerate()
                .map(|(col_idx, cell)| {
                    cell_required_height(arena, cell.node_id, column_widths[col_idx], child_context)
                })
                .max()
                .unwrap_or(1)
                .max(1)
        })
        .collect();

    let mut table_children = Vec::new();
    let mut current_y = outer_y_top + edges.border.top.thickness() + edges.padding.top;
    let content_x = x + edges.border.left.thickness() + edges.padding.left;

    for (row_idx, row) in row_cells.iter().enumerate() {
        let row_height = row_heights[row_idx];
        let mut current_x = content_x;
        for (col_idx, cell) in row.iter().enumerate() {
            let col_w = column_widths[col_idx];
            let cell_rect = Rect::new(current_x, current_y, col_w, row_height);
            let cell_box = layout_cell(
                arena,
                cell.node_id,
                cell_rect,
                cell.is_header,
                child_context,
            );
            table_children.push(cell_box);
            current_x = current_x.saturating_add(col_w);
        }
        current_y = current_y.saturating_add(row_height);
    }

    let total_height = preallocated_height.unwrap_or_else(|| {
        current_y
            .saturating_sub(outer_y_top)
            .saturating_add(edges.border.bottom.thickness())
            .saturating_add(edges.padding.bottom)
    });

    if preallocated_height.is_none() {
        cursor.y = outer_y_top
            .saturating_add(total_height)
            .saturating_add(edges.margin.bottom);
        // Leave the bottom margin for the next in-flow sibling to collapse
        // its top margin against.
        cursor.leave_bottom_margin(edges.margin.bottom);
    }

    let rect_width = preallocated_height
        .map(|_| block_width)
        .unwrap_or(total_table_width);
    let rect = Rect::new(x, outer_y_top, rect_width, total_height);
    let block_bg = node.css_attr.get_bg_color();
    let fill_bg = block_bg.is_some();

    let paint_style = PaintStyle {
        box_edges: edges,
        runs: None,
        column_layout: None,
        font_size_cells: own_font_size_cells,
        line_height_cells: own_font_size_cells.max(1),
        border_radius: crate::html::css_parse::CornerRadii::default(),
        fill_bg,
        spacing_after: 0,
        text_align: node.css_attr.get_text_align(),
    };

    let table_box = LayoutBox::new_normal(
        node_id,
        rect,
        paint_style,
        node.css_attr.outer_display(&node.node_type),
        table_children,
    );

    Some(table_box)
}
