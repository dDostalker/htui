use crossterm::style::Color;

use crate::{
    html::css_parse::TextAlign,
    layout::InlineRun,
    tui::graph_api::{Block, FontStyle, TermLayout, char_width},
};

/// Render each run in `runs` with its own style, colour and background.
///
/// Width accounting uses [`InlineRun::width`] (which sums `char_width` per
/// char) so a wide CJK glyph at the end of one run does not collide with
/// the first cell of the next run. The block's `effective_bg` is applied
/// to runs whose own background is `None`, inheriting the surrounding
/// rect's fill (e.g. an `<aside>`'s `darkblue` strip showing through
/// unstyled text).
///
/// `font_size_cells` is the effective font-size of the containing box. When
/// `font_size_cells > 1`, each character cell is expanded so the character
/// occupies that many terminal columns (the trailing columns are filled with
/// the run's background, mirroring the empty space a larger font would leave
/// around its glyphs).
pub(super) fn paint_runs(
    term_layout: &mut TermLayout,
    runs: &[InlineRun],
    cell_start: usize,
    block_width: usize,
    align: TextAlign,
    effective_bg: Option<Color>,
    font_size_cells: usize,
) {
    let total_width: usize = runs.iter().map(|r| r.width_at(font_size_cells)).sum();
    let total_width = total_width.min(block_width);
    let offset = match align {
        TextAlign::Left => 0,
        TextAlign::Center => block_width.saturating_sub(total_width) / 2,
        TextAlign::Right => block_width.saturating_sub(total_width),
    };
    // The text sits inside the content area (rect shrunk by border + padding);
    // `cell_start` is the first column of the content row and `block_width`
    // is the content-box width. Clip the painted text to this range so a long
    // inline run does not bleed into subsequent rows (or off the right edge
    // of the canvas) and overwrite neighbouring boxes.
    let row_end = cell_start.saturating_add(block_width);
    let mut index = cell_start + offset;
    for run in runs {
        if run.is_empty() {
            continue;
        }
        let run_bg = run.bg.or(effective_bg);
        paint_text_row(
            term_layout,
            index,
            &run.text,
            run.fg,
            run_bg,
            run.style,
            font_size_cells,
            row_end,
        );
        index = index.saturating_add(run.width_at(font_size_cells));
    }
}

fn paint_text_row(
    term_layout: &mut TermLayout,
    start: usize,
    text: &str,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
    font_size_cells: usize,
    row_end: usize,
) {
    let cells = term_layout.layout.as_mut_slice();
    let max = cells.len();
    let mut idx = start;
    let font_size = font_size_cells.max(1);
    for c in text.chars() {
        let w = char_width(c);
        // Stop when the next char + its background padding would step past
        // the content-box right edge or the canvas boundary. Without this,
        // a long `<p>` would spill its overflow into the rows below and
        // corrupt the cells of subsequent boxes.
        let slot_width = w.max(font_size);
        if idx.saturating_add(slot_width) > row_end {
            break;
        }
        if idx >= max {
            break;
        }
        if w == 2 && idx + 1 >= max {
            break;
        }
        cells[idx] = Block::new(c.to_string(), fg, bg, style);
        if w == 2 {
            cells[idx + 1] = Block::new(String::new(), fg, bg, style);
        }
        // Fill the remaining columns the "font" would occupy beyond the glyph
        // itself. Each character cell is expanded to `font_size` columns
        // wide — the glyph itself takes `w` columns (1 for ASCII, 2 for wide
        // CJK), and the rest are filled with space cells carrying the same
        // fg / bg / style so the terminal paints the background across the
        // whole slot. An empty word would be skipped by the renderer
        // (`render_to_bytes` drops cells with `word.is_empty()` so the
        // second half of a wide CJK glyph doesn't double-print), which would
        // erase the padded cell's bg. Wide glyphs still skip the
        // second-column overwrite via the `w == 2 && pad == 1` guard so the
        // paired empty cell stays intact.
        for pad in 1..font_size {
            let target = idx + pad;
            if target >= row_end {
                break;
            }
            if target >= max {
                break;
            }
            if w == 2 && pad == 1 {
                continue;
            }
            cells[target] = Block::new(" ".to_string(), fg, bg, style);
        }
        // Advance to the next character: the slot is `font_size` columns
        // wide (or `w` columns wide, whichever is larger — `w > font_size`
        // for narrow fonts rendering wide glyphs).
        idx += slot_width;
    }
}
