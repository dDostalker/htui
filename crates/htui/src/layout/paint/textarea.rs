//! `<textarea>` painting — a background-filled multi-line editable field with
//! a caret.
//!
//! The value (the node's text content, or the focused edit state) is split
//! into visual lines wrapped to the content width; the box shows a scroll
//! window derived from the caret so the caret always stays visible. The line
//! math here is shared with the input loop (caret placement / up-down
//! movement), which re-imports it through `paint::mod`. No bracket frame; a
//! `border:` declaration opts into a CSS frame via the shared control chrome.

use crossterm::style::Color;

use crate::{
    html::css_parse::parse_color_str,
    layout::{LayoutBox, Rect},
    tdom::node::NodeArena,
    tui::graph_api::{FontStyle, char_width},
};

use super::{PaintCanvas, clamp_rect_to_canvas, intersect_rects, paint_control_chrome};

/// Highlight colour for the selected character range (Tokyo Night selection).
const SELECTION_BG: &str = "#3b4261";

/// One visual line of a `<textarea>` value: the display text wrapped to
/// `content_width` cells, plus the character index in `value` where the line
/// starts (so the caret / selection can map back to global indices).
pub(crate) struct TextareaLine {
    pub char_start: usize,
    pub text: String,
}

/// Split `value` into visual lines: `\n`-separated logical lines, each
/// further wrapped when it overflows `content_width` (CJK glyphs occupy two
/// cells, matching the engine's [`char_width`]).
pub(crate) fn textarea_visual_lines(value: &str, content_width: usize) -> Vec<TextareaLine> {
    let cw = content_width.max(1);
    let mut out = Vec::new();
    let mut char_idx = 0usize;
    for logical in value.split('\n') {
        let chars: Vec<char> = logical.chars().collect();
        let mut line_start = 0usize;
        let mut width = 0usize;
        for (i, &c) in chars.iter().enumerate() {
            let w = char_width(c).max(1);
            if width + w > cw && i > line_start {
                out.push(TextareaLine {
                    char_start: char_idx.saturating_add(line_start),
                    text: chars[line_start..i].iter().collect(),
                });
                line_start = i;
                width = 0;
            }
            width += w;
        }
        out.push(TextareaLine {
            char_start: char_idx.saturating_add(line_start),
            text: chars[line_start..].iter().collect(),
        });
        char_idx = char_idx
            .saturating_add(logical.chars().count())
            .saturating_add(1);
    }
    out
}

/// `(visual row, caret column in cells)` of `cursor` within `value`.
///
/// A cursor at the end of a visual line stays on that line (its column is
/// the line's cell width), so the displayed caret position agrees with the
/// editing layer's character index: `Backspace` removes `cursor - 1` and
/// `Delete` removes `cursor` — exactly the characters either side of the
/// painted caret, including the `\n` between lines.
pub(crate) fn textarea_caret_position(
    value: &str,
    content_width: usize,
    cursor: usize,
) -> (usize, usize) {
    let lines = textarea_visual_lines(value, content_width);
    for (row, line) in lines.iter().enumerate() {
        let end = line.char_start.saturating_add(line.text.chars().count());
        if cursor <= end {
            let col: usize = line
                .text
                .chars()
                .take(cursor.saturating_sub(line.char_start))
                .map(|c| char_width(c).max(1))
                .sum();
            return (row, col);
        }
    }
    (lines.len().saturating_sub(1), 0)
}

/// Character index at visual `(row, col_cells)` — the inverse of
/// [`textarea_caret_position`]. Rows / columns beyond the content clamp to
/// the nearest reachable character.
pub(crate) fn textarea_char_at(
    value: &str,
    content_width: usize,
    row: usize,
    col_cells: usize,
) -> usize {
    let lines = textarea_visual_lines(value, content_width);
    let Some(line) = lines.get(row) else {
        return value.chars().count();
    };
    let mut w = 0usize;
    for (i, c) in line.text.chars().enumerate() {
        if w >= col_cells {
            return line.char_start.saturating_add(i);
        }
        w += char_width(c).max(1);
    }
    line.char_start.saturating_add(line.text.chars().count())
}

/// Scroll row that keeps the caret's visual row inside a `rows`-tall box.
/// Derived from the caret alone (no stored scroll state), so the painter and
/// the input loop always agree.
pub(crate) fn textarea_scroll_row(
    value: &str,
    content_width: usize,
    rows: usize,
    cursor: usize,
) -> usize {
    let (caret_row, _) = textarea_caret_position(value, content_width, cursor);
    let rows = rows.max(1);
    if caret_row.saturating_add(1) > rows {
        caret_row.saturating_add(1).saturating_sub(rows)
    } else {
        0
    }
}

pub(crate) fn paint_textarea(
    lb: &LayoutBox,
    arena: &NodeArena,
    canvas: &mut PaintCanvas<'_>,
    clip: Rect,
) {
    let r = lb.rect;
    let canvas_cols = canvas.columns as usize;
    if r.x >= canvas_cols || r.width == 0 || r.height == 0 {
        return;
    }
    let Some(r) = clamp_rect_to_canvas(r, canvas_cols) else {
        return;
    };
    let Some(visible) = intersect_rects(Rect::new(r.x, r.y, r.width, r.height), clip) else {
        return;
    };

    let node = &arena[lb.node_id];
    // Focused editing state wins; otherwise the value is the node's text
    // content (the textarea's initial value).
    let (value, cursor, focused, selection) = match canvas.input_states.get(&lb.node_id) {
        Some(st) => (st.value.clone(), st.cursor, st.focused, st.selection),
        None => (node.text_content(arena), 0, false, None),
    };

    let fg = node.css_attr.get_color();
    // Background fill (+ opt-in CSS border); the text draws into the returned
    // content rect.
    let content = paint_control_chrome(lb, arena, canvas, r, fg);
    let bg = node.css_attr.get_bg_color().or(Some(super::CONTROL_DEFAULT_BG));
    // `caret-color` paints the caret block with the declared colour; the
    // default is the inverted cell (field background <-> text colour swap).
    let caret_color = node.css_attr.get_caret_color();
    let caret_cell = |fg: Option<Color>, bg: Option<Color>| match caret_color {
        Some(cc) => (Some(bg.unwrap_or(Color::Black)), Some(cc)),
        None => (Some(bg.unwrap_or(Color::Black)), Some(fg.unwrap_or(Color::White))),
    };

    let content_width = content.width.max(1);
    let content_rows = content.height.max(1);
    let lines = textarea_visual_lines(&value, content_width);
    let scroll = textarea_scroll_row(&value, content_width, content_rows, cursor);
    let (caret_row, caret_col) = textarea_caret_position(&value, content_width, cursor);

    for vis_row in 0..content_rows {
        let y = content.y.saturating_add(vis_row);
        if y < visible.y || y >= visible.y.saturating_add(visible.height) {
            continue;
        }
        let Some(line) = lines.get(scroll.saturating_add(vis_row)) else {
            continue;
        };
        let line_row = scroll.saturating_add(vis_row);
        let mut col = 0usize;
        for (i, c) in line.text.chars().enumerate() {
            let w = char_width(c).max(1);
            let global_char = line.char_start.saturating_add(i);
            let is_caret = focused && line_row == caret_row && col == caret_col;
            let (cell_fg, cell_bg) = if is_caret {
                // Inverted caret cell so the position is visible even at the
                // end of the value (a blank inverted cell).
                caret_cell(fg, bg)
            } else if selection.is_some_and(|(s, e)| global_char >= s && global_char < e) {
                (Some(Color::White), parse_color_str(SELECTION_BG))
            } else {
                (fg, bg)
            };
            for sub in 0..w {
                let abs_col = content.x.saturating_add(col).saturating_add(sub);
                if abs_col >= content.x.saturating_add(content_width) {
                    break;
                }
                if abs_col < visible.x || abs_col >= visible.x.saturating_add(visible.width) {
                    continue;
                }
                put(
                    canvas,
                    cell_index(canvas, abs_col, y),
                    if sub == 0 { Some(c) } else { None },
                    cell_fg,
                    cell_bg,
                );
            }
            col += w;
        }
        // Caret past the last character of its visual line: an inverted blank
        // cell at the caret column (kept inside the field).
        if focused && line_row == caret_row && caret_col >= col && caret_col < content_width {
            let abs_col = content.x.saturating_add(caret_col);
            if abs_col < content.x.saturating_add(content_width) {
                let (cf, cb) = caret_cell(fg, bg);
                put(
                    canvas,
                    cell_index(canvas, abs_col, y),
                    Some(' '),
                    cf,
                    cb,
                );
            }
        }
    }
}

fn cell_index(canvas: &PaintCanvas<'_>, x: usize, y: usize) -> usize {
    canvas.cell_index(x, y)
}

fn put(
    canvas: &mut PaintCanvas<'_>,
    idx: usize,
    word: Option<char>,
    fg: Option<Color>,
    bg: Option<Color>,
) {
    if idx >= canvas.layout.len() || !canvas.in_region(idx) {
        return;
    }
    let cell = &mut canvas.layout[idx];
    cell.word.clear();
    if let Some(c) = word {
        cell.word.push(c);
    }
    cell.fcolor = fg;
    cell.bcolor = bg;
    cell.style = FontStyle::empty();
}
