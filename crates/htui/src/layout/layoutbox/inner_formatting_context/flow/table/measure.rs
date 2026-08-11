//! Table measurement: walk the DOM, compute column widths, sum grapheme
//! widths per cell.

use unicode_segmentation::UnicodeSegmentation;

use crate::{
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::grapheme_width,
};

use super::cell::CellInfo;

/// Padding applied to every cell, on each side, unless the cell's CSS
/// overrides it.
///
/// 1 cell of breathing room matches the typical visual rhythm of a
/// bordered table in a terminal — narrower cells become unreadable, wider
/// cells waste horizontal space.
pub(super) const CELL_PADDING: usize = 1;

/// Minimum width of any column, in cells.
///
/// Three cells is enough to render short tokens like "ID" or "n/a" without
/// the column collapsing to a single character that wraps awkwardly.
pub(super) const MIN_COLUMN_WIDTH: usize = 3;

/// Walk the table's child list and return every `<tr>` node ID in document
/// order, flattening through `<thead>` / `<tbody>` / `<tfoot>` wrappers.
pub(super) fn collect_rows(arena: &NodeArena, table_id: NodeId) -> Vec<NodeId> {
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
pub(super) fn collect_cells(arena: &NodeArena, row_id: NodeId) -> Vec<CellInfo> {
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
pub(super) fn cell_text_width(arena: &NodeArena, cell_id: NodeId) -> usize {
    let node = &arena[cell_id];
    let mut total = 0usize;
    for &child_id in &node.children {
        if let NodeType::Text = arena[child_id].node_type {
            if let Some(s) = arena[child_id].data.as_deref() {
                total += s.graphemes(true).map(grapheme_width).sum::<usize>();
            }
        }
    }
    total
}

/// Compute the column widths for a table.
///
/// Each column adopts the maximum content width of any cell in that column
/// (plus [`CELL_PADDING`] on each side), clamped to [`MIN_COLUMN_WIDTH`].
/// If the resulting total exceeds `available_width`, columns are scaled
/// proportionally; any rounded-down remainder is distributed one cell at a
/// time to the widest columns to keep total widths exact.
pub(super) fn compute_column_widths(
    arena: &NodeArena,
    row_cells: &[Vec<CellInfo>],
    available_width: usize,
) -> Vec<usize> {
    let column_count = row_cells.iter().map(|row| row.len()).max().unwrap_or(0);
    if column_count == 0 {
        return Vec::new();
    }
    let mut widths = vec![0usize; column_count];
    for row in row_cells {
        for (col_idx, cell) in row.iter().enumerate() {
            let w = cell_text_width(arena, cell.node_id) + CELL_PADDING * 2;
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
    if total > available_width && total > 0 {
        let mut scaled: Vec<usize> = widths
            .iter()
            .map(|w| w * available_width / total)
            .collect();
        let scaled_total: usize = scaled.iter().sum();
        let mut remainder = available_width.saturating_sub(scaled_total);
        let mut indices: Vec<usize> = (0..column_count).collect();
        indices.sort_by(|a, b| widths[*b].cmp(&widths[*a]));
        let mut idx_pos = 0;
        while remainder > 0 {
            let i = indices[idx_pos % column_count];
            scaled[i] = scaled[i].saturating_add(1);
            remainder -= 1;
            idx_pos += 1;
        }
        return scaled;
    }
    widths
}
