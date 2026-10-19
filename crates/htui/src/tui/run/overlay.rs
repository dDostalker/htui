//! `<select>` dropdown overlay painting onto the canvas.

use crate::layout::layoutbox::geometry::SelectRow;
use crate::tui::graph_api::{Block, FontStyle, TermLayout, char_width};

use super::state::{GROUP_FG, OVERLAY_BG, OVERLAY_FG, OpenSelect, SELECTION_BG};

/// Paint an open `<select>` dropdown overlay onto the canvas: clear every
/// row of the overlay rectangle, then write each option label with the
/// highlighted row on the selection background and the rest on the overlay
/// background. Overlays cover whatever the layout painted underneath.
pub(super) fn apply_select_overlay(tl: &mut TermLayout, open: &OpenSelect) {
    let cols = tl.columns as usize;
    let max = tl.layout.len();
    // The overlay may extend below the layout's content (a `<select>` near
    // the bottom of a short document): grow the canvas buffer so every
    // option row is paintable. This does not touch `rendered_rows`, so the
    // overlay never affects the scroll range.
    let needed = open
        .rect
        .y
        .saturating_add(open.rect.height)
        .saturating_mul(cols);
    if needed > max {
        tl.layout.resize(needed, Block::default());
    }
    let max = tl.layout.len();
    let x_end = open.rect.x.saturating_add(open.rect.width).min(cols);
    for (row_idx, row) in open.rows.iter().enumerate() {
        let row_start = open.rect.y.saturating_add(row_idx).saturating_mul(cols);
        if row_start >= max {
            break;
        }
        let (label, fg, is_item) = match row {
            // `<optgroup>` headers are dimmed and never take the highlight.
            SelectRow::Group(g) => (g.as_str(), GROUP_FG, false),
            SelectRow::Item(i) => (
                open.options.get(*i).map(|o| o.label.as_str()).unwrap_or(""),
                OVERLAY_FG,
                true,
            ),
        };
        let bg = if is_item && row_idx == open.highlight_row() {
            Some(SELECTION_BG)
        } else {
            Some(OVERLAY_BG)
        };
        // Clear the whole row, then write the label.
        for x in open.rect.x..x_end {
            let idx = row_start.saturating_add(x);
            if idx >= max {
                break;
            }
            let cell = &mut tl.layout[idx];
            cell.word.clear();
            cell.word.push(' ');
            cell.fcolor = Some(fg);
            cell.bcolor = bg;
            cell.style = FontStyle::empty();
        }
        let mut col = open.rect.x;
        for ch in label.chars() {
            let w = char_width(ch).max(1);
            if col >= x_end {
                break;
            }
            let idx = row_start.saturating_add(col);
            if idx < max {
                let cell = &mut tl.layout[idx];
                cell.word.clear();
                cell.word.push(ch);
                cell.fcolor = Some(fg);
                cell.bcolor = bg;
                cell.style = FontStyle::empty();
            }
            col = col.saturating_add(w);
        }
    }
}
