//! Page text selection: per-row line bands, highlight bounds and text
//! extraction from the layout inline runs.

use crate::layout::{LayoutBox, Rect};
use crate::tdom::node::NodeArena;
use crate::tui::graph_api::SelectionMode;

use super::state::TextSel;

/// The per-row column bands a selection covers — the single source of truth
/// for both the highlight and the extracted text. Each entry is
/// `(row, col_start, col_end_exclusive)`.
///
/// Flow (left-drag) follows reading order: the anchor row runs from the
/// anchor column to the right edge, the end row from the left edge to the end
/// column, and every row in between is full-width. Block (right-drag) is a
/// plain rectangle.
pub(super) fn selection_lines(sel: &TextSel, cols: u32) -> Vec<(u32, u32, u32)> {
    let (ax, ay) = sel.anchor;
    let (ex, ey) = sel.end;
    match sel.mode {
        SelectionMode::Block => {
            let x1 = ax.min(ex);
            let x2 = ax.max(ex).saturating_add(1);
            let y1 = ay.min(ey);
            let y2 = ay.max(ey).saturating_add(1);
            (y1..y2).map(|y| (y, x1, x2)).collect()
        }
        SelectionMode::Flow => {
            if ay == ey {
                vec![(ay, ax.min(ex), ax.max(ex).saturating_add(1))]
            } else if ay < ey {
                let mut lines = vec![(ay, ax, cols)];
                for y in ay.saturating_add(1)..ey {
                    lines.push((y, 0, cols));
                }
                lines.push((ey, 0, ex.saturating_add(1)));
                lines
            } else {
                // Dragged upward: the end point sits above the anchor.
                let mut lines = vec![(ey, ex, cols)];
                for y in ey.saturating_add(1)..ay {
                    lines.push((y, 0, cols));
                }
                lines.push((ay, 0, ax.saturating_add(1)));
                lines
            }
        }
    }
}

/// Bounding rectangle of a set of line bands (used to pick the repaint
/// region; the bands themselves drive the actual highlighting).
pub(super) fn lines_bounds(lines: &[(u32, u32, u32)]) -> Option<Rect> {
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    for &(y, xs, xe) in lines {
        min_y = min_y.min(y);
        min_x = min_x.min(xs);
        max_y = max_y.max(y);
        max_x = max_x.max(xe);
    }
    if max_y < min_y {
        return None;
    }
    Some(Rect::new(
        min_x as usize,
        min_y as usize,
        (max_x - min_x) as usize,
        (max_y - min_y + 1) as usize,
    ))
}

pub(super) fn highlight_lines(
    arena: &NodeArena,
    boxes: &[LayoutBox],
    sel: &TextSel,
    cols: u32,
) -> Vec<(u32, u32, u32)> {
    let raw = selection_lines(sel, cols);
    match sel.mode {
        SelectionMode::Flow => {
            // Keep the drag inside the block it started in: a vertical drag
            // in one column must not highlight a side-by-side card's text.
            let raw = crate::layout::clip_flow_lines_scoped(boxes, sel.anchor, sel.end, &raw);
            crate::layout::selection_text_lines(arena, boxes, &raw)
        }
        SelectionMode::Block => raw,
    }
}
