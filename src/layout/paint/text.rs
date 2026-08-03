use crossterm::style::Color;
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    html::css_parse::TextAlign,
    layout::{InlineRun, Rect},
    tui::graph_api::{FontStyle, grapheme_width},
};

use super::PaintCanvas;

/// Render each visual line in `lines` with its own style, colour and
/// background.
///
/// `lines` is the result of [`crate::layout::layoutbox::wrap_runs`]:
/// each inner `Vec<InlineRun>` is one visual line that fits within
/// the content-box width after wrapping. This entry point lays out
/// every line top-to-bottom, advancing `line_height_cells * canvas.columns`
/// cells per line so each row of the rect paints a single visual line
/// (`line_height_cells` rows tall; `normal` line-height equals
/// `font_size_cells`).
///
/// Per-line width accounting uses [`InlineRun::width_at`] (which sums
/// `max(grapheme_width, font_size)` per grapheme) so a wide CJK glyph
/// at the end of one run does not collide with the first cell of the
/// next run. The block's `effective_bg` is applied to runs whose own
/// background is `None`, inheriting the surrounding rect's fill (e.g.
/// an `<aside>`'s `darkblue` strip showing through unstyled text).
///
/// `font_size_cells` is the effective font-size of the containing box.
/// When `font_size_cells > 1`, each character cell is expanded so the
/// character occupies that many terminal columns.
pub(crate) fn paint_runs(
    canvas: &mut PaintCanvas<'_>,
    lines: &[Vec<InlineRun>],
    cell_start: usize,
    block_width: usize,
    align: TextAlign,
    effective_bg: Option<Color>,
    font_size_cells: usize,
    line_height_cells: usize,
    clip: Rect,
) {
    if lines.is_empty() {
        return;
    }
    let cols = canvas.columns as usize;
    // The number of canvas rows one visual line occupies (CSS `line-height`,
    // resolved to whole rows). `normal` yields `font`, so the advance is the
    // font-size unless the author enlarged the line box.
    let line_h = line_height_cells.max(1);
    // The canvas row the first visual line's glyphs sit on. Each visual
    // line advances `line_h` canvas rows below it.
    let first_line_row = cell_start / cols;
    for (line_idx, line_runs) in lines.iter().enumerate() {
        // Drop lines whose glyph row falls outside the inherited clip
        // (e.g. wrapped content of a box that was squeezed smaller than
        // its text on a very small terminal).
        let line_row = first_line_row.saturating_add(line_idx.saturating_mul(line_h));
        if line_row < clip.y || line_row >= clip.y.saturating_add(clip.height) {
            continue;
        }
        if line_runs.is_empty() {
            // Forced-break blank row (e.g. a `<pre>` newline); the bg-fill
            // loop in `paint_block` already covers it, so skip writing
            // any text here.
            continue;
        }
        let line_start =
            cell_start.saturating_add(line_idx.saturating_mul(line_h).saturating_mul(cols));
        paint_one_line(
            canvas,
            line_runs,
            line_start,
            block_width,
            align,
            effective_bg,
            font_size_cells,
        );
    }
}

/// Render a single visual line's runs.
fn paint_one_line(
    canvas: &mut PaintCanvas<'_>,
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
            canvas,
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
    canvas: &mut PaintCanvas<'_>,
    start: usize,
    text: &str,
    fg: Option<Color>,
    bg: Option<Color>,
    style: FontStyle,
    font_size_cells: usize,
    row_end: usize,
) {
    let max = canvas.layout.len();
    let mut idx = start;
    let font_size = font_size_cells.max(1);
    for g in text.graphemes(true) {
        let w = grapheme_width(g);
        // Stop when the next cluster + its background padding would step
        // past the content-box right edge or the canvas boundary. Without
        // this, a long `<p>` would spill its overflow into the rows below
        // and corrupt the cells of subsequent boxes.
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
        // In-place mutation: the buffer from `Block::default()` (32 bytes
        // by default) fits any single grapheme cluster so `push_str(g)`
        // reuses it without reallocating.
        let cell = &mut canvas.layout[idx];
        cell.word.clear();
        cell.word.push_str(g);
        cell.fcolor = fg;
        cell.bcolor = bg;
        cell.style = style;
        if w == 2 {
            // Wide clusters (CJK ideographs, 2-column emoji, ZWJ ligatures)
            // occupy two columns; the second column is marked with an empty
            // word so the renderer (`render_to_bytes`) skips it and doesn't
            // double-print the glyph.
            let pad = &mut canvas.layout[idx + 1];
            pad.word.clear();
            pad.fcolor = fg;
            pad.bcolor = bg;
            pad.style = style;
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
            let pad_cell = &mut canvas.layout[target];
            pad_cell.word.clear();
            pad_cell.word.push(' ');
            pad_cell.fcolor = fg;
            pad_cell.bcolor = bg;
            pad_cell.style = style;
        }
        // Advance to the next cluster: the slot is `font_size` columns wide
        // (or `w` columns wide, whichever is larger — `w > font_size` for
        // narrow fonts rendering wide glyphs).
        idx += slot_width;
    }
}
