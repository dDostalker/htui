//! Layout the `<table>` element itself.

use crate::{
    layout::{
        BoxConstraint, Cursor, FontContext, LayoutBox, PaintStyle, Rect,
    },
    tdom::node::{NodeArena, NodeId, NodeType, effective_font_size_cells},
};

use super::cell::{CellInfo, layout_cell};
use super::measure::{collect_cells, collect_rows, compute_column_widths};

/// Build a [`LayoutBox`] for a `<table>` element.
pub fn layout_table(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
    pre_allocated_rect: Option<Rect>,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Table {
        return None;
    }

    let own_font_size_cells = effective_font_size_cells(
        arena,
        node_id,
        font_ctx.font_size_cells,
        font_ctx.root_font_size_cells,
    );
    let child_font_ctx = font_ctx.child(own_font_size_cells);

    let edges = node.css_attr.get_box_edges(
        containing.width,
        own_font_size_cells,
        font_ctx.root_font_size_cells,
    );

    let available = containing.width.saturating_sub(edges.margin.horizontal());
    let (x, block_width, outer_y_top, preallocated_height) = match pre_allocated_rect {
        Some(rect) => (rect.x, rect.width.max(1), rect.y, Some(rect.height.max(1))),
        None => {
            let block_width = node
                .css_attr
                .get_width(
                    available,
                    own_font_size_cells,
                    font_ctx.root_font_size_cells,
                )
                .unwrap_or(available)
                .min(available)
                .max(1);
            cursor.skip(edges.margin.top);
            (containing.x + edges.margin.left, block_width, cursor.y, None)
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

    let row_height = 1usize;
    let mut table_children = Vec::new();
    let mut current_y = outer_y_top + edges.border.top.thickness() + edges.padding.top;
    let content_x = x + edges.border.left.thickness() + edges.padding.left;

    for row in &row_cells {
        let mut current_x = content_x;
        for (col_idx, cell) in row.iter().enumerate() {
            let col_w = column_widths[col_idx];
            let cell_rect = Rect::new(current_x, current_y, col_w, row_height);
            let cell_box = layout_cell(
                arena,
                cell.node_id,
                cell_rect,
                cell.is_header,
                child_font_ctx,
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
        font_size_cells: own_font_size_cells,
        fill_bg,
        spacing_after: 0,
        chrome: false,
    };

    let table_box = LayoutBox::new_normal(
        node_id,
        rect,
        paint_style,
        node.css_attr.outer_display(),
        table_children,
    );

    Some(table_box)
}
