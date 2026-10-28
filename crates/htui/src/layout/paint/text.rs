use crossterm::style::Color;
use unicode_segmentation::UnicodeSegmentation;

use crate::{
    graphics::{FontStyle, grapheme_width},
    html::css_parse::TextAlign,
    layout::{InlineRun, Rect},
};

use super::{BackgroundFill, PaintCanvas, background_at, text_color_at};

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
/// next run. The box's `effective_bg` is applied to runs whose own
/// background is `None`, inheriting the surrounding rect's fill (e.g.
/// an `<aside>`'s `darkblue` strip showing through unstyled text).
///
/// `font_size_cells` is the effective font-size of the containing box.
/// When `font_size_cells > 1`, each character cell is expanded so the
/// character occupies that many terminal columns.
///
/// `overflow_visible` extends the paint bounds past the box's content box
/// (CSS `overflow: visible`): lines paint beyond the rect's height (bounded
/// by the inherited clip) and each row's glyph stream runs to the canvas
/// edge instead of stopping at the content-box width. Clipping boxes keep
/// the content-box bounds.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_runs(
    canvas: &mut PaintCanvas<'_>,
    lines: &[Vec<InlineRun>],
    cell_start: usize,
    block_width: usize,
    align: TextAlign,
    effective_bg: Option<BackgroundFill<'_>>,
    text_fill: Option<BackgroundFill<'_>>,
    font_size_cells: usize,
    line_height_cells: usize,
    clip: Rect,
    h_bound_cols: usize,
    first_line_indent: usize,
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
        // The bound is row-relative (columns from the row start), so every
        // visual line resolves it against its own row.
        let line_row_base = line_start.saturating_sub(line_start % cols);
        let h_end = line_row_base.saturating_add(h_bound_cols.min(cols));
        // `text-indent` shifts the first visual line right and narrows its
        // alignment basis (the wrap pass already reserved the budget).
        let indent = if line_idx == 0 { first_line_indent } else { 0 };
        let indent = indent.min(block_width);
        paint_one_line(
            canvas,
            line_runs,
            line_start.saturating_add(indent),
            block_width.saturating_sub(indent),
            align,
            effective_bg,
            text_fill,
            font_size_cells,
            h_end,
        );
    }
}

/// Render a single visual line's runs.
///
/// `h_end` is the exclusive flat-index bound the glyph stream stops at —
/// the caller decides the overflow policy (content-box edge for clipping
/// boxes, the frame edge for a visibly-overflowing bordered box, the row's
/// canvas edge for a borderless one).
#[allow(clippy::too_many_arguments)]
fn paint_one_line(
    canvas: &mut PaintCanvas<'_>,
    runs: &[InlineRun],
    cell_start: usize,
    block_width: usize,
    align: TextAlign,
    effective_bg: Option<BackgroundFill<'_>>,
    text_fill: Option<BackgroundFill<'_>>,
    font_size_cells: usize,
    h_end: usize,
) {
    let total_width: usize = runs.iter().map(|r| r.width_at(font_size_cells)).sum();
    let total_width = total_width.min(block_width);
    let offset = match align {
        TextAlign::Left => 0,
        TextAlign::Center => block_width.saturating_sub(total_width) / 2,
        TextAlign::Right => block_width.saturating_sub(total_width),
    };
    let row_end = h_end;
    let mut index = cell_start + offset;
    for run in runs {
        if run.is_empty() {
            continue;
        }
        paint_text_row(
            canvas,
            index,
            &run.text,
            run.fg,
            run.bg,
            effective_bg,
            text_fill,
            run.style,
            font_size_cells,
            row_end,
            run.letter_spacing,
            run.word_spacing,
        );
        index = index.saturating_add(run.width_at(font_size_cells));
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_text_row(
    canvas: &mut PaintCanvas<'_>,
    start: usize,
    text: &str,
    fg: Option<Color>,
    run_bg: Option<Color>,
    inherited_bg: Option<BackgroundFill<'_>>,
    text_fill: Option<BackgroundFill<'_>>,
    style: FontStyle,
    font_size_cells: usize,
    row_end: usize,
    letter_spacing: usize,
    word_spacing: isize,
) {
    let max = canvas.layout.len();
    let cols = canvas.columns as usize;
    let mut idx = start;
    let font_size = font_size_cells.max(1);
    for g in text.graphemes(true) {
        let w = grapheme_width(g);
        // Same advance formula as `InlineRun::grapheme_advance` so the
        // painted cells line up with every width measurement: the glyph's
        // expanded slot plus word spacing on spaces plus letter spacing.
        let word_extra = if g == " " {
            word_spacing.max(0) as usize
        } else {
            0
        };
        let slot_width = w.max(font_size) + word_extra + letter_spacing;
        // Stop when the next cluster + its background padding would step
        // past the content-box right edge or the canvas boundary. Without
        // this, a long `<p>` would spill its overflow into the rows below
        // and corrupt the cells of subsequent boxes.
        if idx.saturating_add(slot_width) > row_end {
            break;
        }
        if idx >= max {
            break;
        }
        if w == 2 && idx + 1 >= max {
            break;
        }
        if !canvas.in_region(idx) {
            // Incremental repaint: the glyph sits outside the repaint region
            // (e.g. an ancestor's run that merely touches the region), so it
            // must not be rewritten. Advance past the slot regardless so
            // later clusters keep their positions.
            idx += slot_width;
            continue;
        }
        // In-place mutation: the buffer from `Block::default()` (32 bytes
        // by default) fits any single grapheme cluster so `push_str(g)`
        // reuses it without reallocating.
        let cell = &mut canvas.layout[idx];
        cell.word.clear();
        cell.word.push_str(g);
        cell.fcolor = text_color_at(fg, text_fill, idx % cols, idx / cols);
        cell.bcolor = run_bg.or_else(|| background_at(inherited_bg, idx % cols, idx / cols));
        cell.style = style;
        if w == 2 {
            // Wide clusters (CJK ideographs, 2-column emoji, ZWJ ligatures)
            // occupy two columns; the second column is marked with an empty
            // word so the renderer (`render_to_bytes`) skips it and doesn't
            // double-print the glyph.
            if canvas.in_region(idx + 1) {
                let pad = &mut canvas.layout[idx + 1];
                pad.word.clear();
                pad.fcolor = text_color_at(fg, text_fill, (idx + 1) % cols, (idx + 1) / cols);
                pad.bcolor = run_bg
                    .or_else(|| background_at(inherited_bg, (idx + 1) % cols, (idx + 1) / cols));
                pad.style = style;
            }
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
            if !canvas.in_region(target) {
                continue;
            }
            let pad_cell = &mut canvas.layout[target];
            pad_cell.word.clear();
            pad_cell.word.push(' ');
            pad_cell.fcolor = text_color_at(fg, text_fill, target % cols, target / cols);
            pad_cell.bcolor =
                run_bg.or_else(|| background_at(inherited_bg, target % cols, target / cols));
            pad_cell.style = style;
        }
        // Advance to the next cluster: the slot is `font_size` columns wide
        // (or `w` columns wide, whichever is larger — `w > font_size` for
        // narrow fonts rendering wide glyphs).
        idx += slot_width;
    }
}
