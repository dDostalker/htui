//! Table layout.
//!
//! Produces a 2-D grid of cells from a `<table>` subtree. Layout walks the
//! table once to measure column widths (max content width per column across
//! all rows), then lays out each cell at an explicit
//! `(x, y, width, height)` rect so every column in a row shares the same
//! starting `x` and ending `width` — matching the columnar alignment a
//! `<table>` requires.
//!
//! ## Scope
//!
//! - Supported elements: `<table>`, `<thead>`, `<tbody>`, `<tfoot>`, `<tr>`,
//!   `<th>`, `<td>`. `<th>` is rendered like `<td>`; the user-agent default
//!   `font-weight: bold; text-align: center;` is reproduced either by the
//!   user adding CSS (`th { ... }`) or by the cascade step.
//! - `<caption>`, `<col>`, `<colgroup>` are parsed into the DOM but act as
//!   sinks at layout time (no boxes, no rows).
//! - `<thead>` / `<tbody>` / `<tfoot>` are transparent row-grouping wrappers
//!   — their children are flattened into the table's row list.
//! - `colspan` / `rowspan` are ignored: each cell occupies one column and
//!   one row.
//!
//! ## Cursor management
//!
//! The table block consumes `font_ctx.font_size_cells` to compute its own
//! font-size and forwards the resulting context to every cell. The outer
//! cursor is advanced by `margin.top + total_height + margin.bottom` so the
//! next sibling starts at the correct row.

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    layout::{
        BoxConstraint, BoxEdges, Cursor, FontContext, LayoutBox, Rect,
        tree::{LayoutBlockOptions, collect_runs, layout_block, layout_children},
    },
    tdom::node::{NodeArena, NodeId, NodeType, effective_font_size_cells},
    tui::graph_api::{block_style, grapheme_width},
};

/// Padding applied to every cell, on each side, unless the cell's CSS
/// overrides it.
///
/// 1 cell of breathing room matches the typical visual rhythm of a
/// bordered table in a terminal — narrower cells become unreadable, wider
/// cells waste horizontal space.
const CELL_PADDING: usize = 1;

/// Minimum width of any column, in cells.
///
/// Three cells is enough to render short tokens like "ID" or "n/a" without
/// the column collapsing to a single character that wraps awkwardly.
const MIN_COLUMN_WIDTH: usize = 3;

/// Build a [`LayoutBox`] for a `<table>` element.
///
/// Tables are block containers that lay out their child `<tr>` rows into a
/// column-aligned grid. Column widths are derived from the widest cell in
/// each column (plus [`CELL_PADDING`] each side). The table's own CSS
/// `width`, `padding`, `border`, and `margin` are honoured; the inner
/// and outer rects are computed by the same code path used for `<div>`.
/// Header cells (`<th>`) render as plain cells — users can apply
/// `font-weight: bold; text-align: center;` via CSS to match the browser
/// default.
pub fn layout_table(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
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
    let x = containing.x + edges.margin.left;

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

    cursor.skip(edges.margin.top);
    let outer_y_top = cursor.y;

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

    let total_height = current_y
        .saturating_sub(outer_y_top)
        .saturating_add(edges.border.bottom.thickness())
        .saturating_add(edges.padding.bottom);

    cursor.y = outer_y_top
        .saturating_add(total_height)
        .saturating_add(edges.margin.bottom);

    let rect = Rect::new(x, outer_y_top, total_table_width, total_height);
    let block_bg = node.css_attr.get_bg_color();
    let fill_bg = block_bg.is_some();

    let table_box = LayoutBox::new_block(
        node_id,
        rect,
        edges,
        fill_bg,
        0,
        vec![],
        table_children,
        own_font_size_cells,
    );

    Some(table_box)
}

/// Build a [`LayoutBox`] for a `<tr>` element.
///
/// `<tr>` is a row container inside a `<table>`. The dispatcher reaches it
/// only when the row is misplaced outside a table — in that case it is
/// rendered as a plain block so the row's text content is not silently
/// dropped. Inside a table, the `<tr>` is consumed by [`layout_table`]
/// and never reaches this function.
pub fn layout_tr(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Tr {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions::default(),
        font_ctx,
    ))
}

/// Build a [`LayoutBox`] for a `<th>` element.
///
/// `<th>` is a header cell inside a `<tr>`. Inside a table the cell is
/// laid out by [`layout_table`]; outside a table it falls back to a plain
/// block so the heading text still renders.
pub fn layout_th(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Th {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions::default(),
        font_ctx,
    ))
}

/// Build a [`LayoutBox`] for a `<td>` element.
///
/// `<td>` is a data cell inside a `<tr>`. Inside a table the cell is
/// laid out by [`layout_table`]; outside a table it falls back to a plain
/// block so the cell text still renders.
pub fn layout_td(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if node.node_type != NodeType::Td {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions::default(),
        font_ctx,
    ))
}

/// Build a [`LayoutBox`] for a `<thead>` / `<tbody>` / `<tfoot>` element.
///
/// `<thead>` / `<tbody>` / `<tfoot>` are row-grouping wrappers inside a
/// `<table>`. Inside a table their children are flattened into the table's
/// row list by [`layout_table`]; outside a table they fall back to a plain
/// block so the wrapped content still renders.
pub fn layout_row_group(
    arena: &NodeArena,
    node_id: NodeId,
    containing: BoxConstraint,
    cursor: &mut Cursor,
    font_ctx: FontContext,
) -> Option<LayoutBox> {
    let node = &arena[node_id];
    if !matches!(
        node.node_type,
        NodeType::Thead | NodeType::Tbody | NodeType::Tfoot
    ) {
        return None;
    }

    Some(layout_block(
        arena,
        node,
        node_id,
        containing,
        cursor,
        LayoutBlockOptions::default(),
        font_ctx,
    ))
}

/// Information about a single cell inside a row.
#[derive(Clone, Copy)]
struct CellInfo {
    node_id: NodeId,
    /// `true` for `<th>` cells; the font-style / alignment defaults are
    /// surfaced for callers that want to apply them.
    is_header: bool,
}

/// Walk the table's child list and return every `<tr>` node ID in document
/// order, flattening through `<thead>` / `<tbody>` / `<tfoot>` wrappers.
fn collect_rows(arena: &NodeArena, table_id: NodeId) -> Vec<NodeId> {
    let mut rows = Vec::new();
    for &child_id in &arena[table_id].children {
        match arena[child_id].node_type {
            NodeType::Tr => rows.push(child_id),
            NodeType::Thead | NodeType::Tbody | NodeType::Tfoot => {
                for &grandchild_id in &arena[child_id].children {
                    if arena[grandchild_id].node_type == NodeType::Tr {
                        rows.push(grandchild_id);
                    }
                }
            }
            _ => {}
        }
    }
    rows
}

/// Walk a row's child list and return every `<th>` / `<td>` cell in
/// document order.
fn collect_cells(arena: &NodeArena, row_id: NodeId) -> Vec<CellInfo> {
    let mut cells = Vec::new();
    for &child_id in &arena[row_id].children {
        match arena[child_id].node_type {
            NodeType::Td => cells.push(CellInfo {
                node_id: child_id,
                is_header: false,
            }),
            NodeType::Th => cells.push(CellInfo {
                node_id: child_id,
                is_header: true,
            }),
            _ => {}
        }
    }
    cells
}

/// Measure a cell's inline text width as the sum of `grapheme_width` of every
/// grapheme cluster in every direct text descendant. Nested block elements
/// are ignored — their own cell layout would require its own metric pass and
/// we don't currently surface that.
///
/// Iterating graphemes (rather than `char`s) keeps multi-codepoint emoji
/// (ZWJ ligatures, modifier sequences, variation-selector pairs) at their
/// correct column count — `char_width` would either double-count VS-16 or
/// under-count ZWJ sequences.
fn cell_text_width(arena: &NodeArena, cell_id: NodeId) -> usize {
    let node = &arena[cell_id];
    let mut total = 0usize;
    for &child_id in &node.children {
        if let NodeType::Text(s) = &arena[child_id].node_type {
            total += s.graphemes(true).map(grapheme_width).sum::<usize>();
        }
    }
    total
}

/// Compute the column widths for a table.
///
/// Each column adopts the maximum content width of any cell in that column
/// (plus [`CELL_PADDING`] on each side), clamped to [`MIN_COLUMN_WIDTH`].
/// If the resulting total exceeds `available_width`, columns are scaled
/// proportionally; any rounded-down remainder is recovered by widening the
/// first column so the table exactly fills the available space.
fn compute_column_widths(
    arena: &NodeArena,
    row_cells: &[Vec<CellInfo>],
    available_width: usize,
) -> Vec<usize> {
    let num_cols = row_cells.iter().map(|r| r.len()).max().unwrap_or(0);
    let mut widths = vec![0usize; num_cols];
    for row in row_cells {
        for (col_idx, cell) in row.iter().enumerate() {
            let w = cell_text_width(arena, cell.node_id).saturating_add(2 * CELL_PADDING);
            if w > widths[col_idx] {
                widths[col_idx] = w;
            }
        }
    }
    for w in widths.iter_mut() {
        if *w < MIN_COLUMN_WIDTH {
            *w = MIN_COLUMN_WIDTH;
        }
    }

    let total: usize = widths.iter().sum();
    if total > available_width && total > 0 && !widths.is_empty() {
        let scale = available_width as f64 / total as f64;
        let mut scaled: Vec<usize> = widths
            .iter()
            .map(|w| ((*w as f64) * scale).floor() as usize)
            .collect();
        let remainder = available_width.saturating_sub(scaled.iter().sum::<usize>());
        // Distribute any floor-truncation remainder to the first column so the
        // table fully fills the available width without rounding gaps.
        if remainder > 0 {
            scaled[0] = scaled[0].saturating_add(remainder);
        }
        widths = scaled;
    }

    widths
}

/// Build a single cell's [`LayoutBox`] at the given `rect`.
///
/// Mirrors the body of [`layout_block`] but skips the cursor reservation
/// (the cell's row already pinned its `y`) and lays out nested children
/// inside the cell's content area. Cell-local CSS rules
/// (`padding` / `border` / `background-color` / `color`) are honoured so
/// tables can carry per-cell styling.
fn layout_cell(
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

    LayoutBox::new_block(
        cell_id,
        rect,
        cell_edges,
        fill_bg,
        0,
        body_runs,
        children,
        own_font_size_cells,
    )
}
