//! Input-field painting — a single-line `[value]` field with an optional
//! caret cell and a focused frame colour.

use crossterm::style::Color;

use crate::{
    html::css_parse::parse_color_str,
    layout::{LayoutBox, Rect},
    tdom::node::NodeArena,
    tui::graph_api::{FontStyle, char_width},
};

use super::{PaintCanvas, intersect_rects};

/// Frame colour for a focused field (Tokyo Night accent cyan).
const FOCUS_FRAME: &str = "#2ac3de";

/// Highlight colour for the selected character range (Tokyo Night selection).
const SELECTION_BG: &str = "#3b4261";

pub(crate) fn paint_input(lb: &LayoutBox, arena: &NodeArena, canvas: &mut PaintCanvas<'_>, clip: Rect) {
    let r = lb.rect;
    let canvas_cols = canvas.columns as usize;
    if r.x >= canvas_cols || r.width == 0 || r.height == 0 {
        return;
    }
    let Some(visible) = intersect_rects(Rect::new(r.x, r.y, r.width, r.height), clip) else {
        return;
    };

    let node = &arena[lb.node_id];
    // Engine edit state (typed while focused) wins over the DOM `value`
    // attribute (the initial content of an untouched field).
    let (value, cursor, focused, selection) = match canvas.input_states.get(&lb.node_id) {
        Some(st) => (st.value.clone(), st.cursor, st.focused, st.selection),
        None => (
            node.attributes.get("value").cloned().unwrap_or_default(),
            0,
            false,
            None,
        ),
    };

    let fg = node.css_attr.get_color();
    let bg = node.css_attr.get_bg_color();
    let frame = if focused {
        parse_color_str(FOCUS_FRAME).or(fg)
    } else {
        fg
    };

    let inner_width = r.width.saturating_sub(2);
    let inner_x = r.x.saturating_add(1);

    // Frame brackets.
    put(canvas, cell_index(canvas, r.x, r.y), Some('['), frame, bg);
    put(
        canvas,
        cell_index(canvas, r.x.saturating_add(r.width).saturating_sub(1), r.y),
        Some(']'),
        frame,
        bg,
    );

    // Value window in *display columns* (CJK characters occupy two cells):
    // walk back from the caret until the width before it plus the caret's own
    // column fits, then extend forward while columns remain. This keeps the
    // caret visible without letting a multi-column glyph push past the `]`.
    let v: Vec<char> = value.chars().collect();
    let total = v.len();
    let cursor = cursor.min(total);
    let widths: Vec<usize> = v.iter().map(|&c| char_width(c).max(1)).collect();
    let caret_w: usize = widths.get(cursor).copied().unwrap_or(1);

    let mut start = cursor;
    let mut before_w = 0usize;
    while start > 0 && before_w.saturating_add(widths[start - 1]).saturating_add(caret_w) <= inner_width {
        before_w += widths[start - 1];
        start -= 1;
    }

    let mut end = cursor.saturating_add(1).min(total);
    let mut used = before_w.saturating_add(caret_w);
    while end < total && used.saturating_add(widths[end]) <= inner_width {
        used += widths[end];
        end += 1;
    }

    let mut col = 0usize;
    for i in start..end {
        let ch = v[i];
        let w = widths[i];
        let inverted = focused && i == cursor;
        for sub in 0..w {
            let abs_col = inner_x.saturating_add(col).saturating_add(sub);
            if abs_col >= inner_x.saturating_add(inner_width) {
                break;
            }
            if abs_col < visible.x || abs_col >= visible.x.saturating_add(visible.width) {
                continue;
            }
            let (cell_fg, cell_bg) = if inverted {
                // Inverted caret cell so the caret position is visible even at
                // the end of the value (a blank inverted cell).
                (
                    Some(bg.unwrap_or(Color::Black)),
                    Some(fg.unwrap_or(Color::White)),
                )
            } else if selection.is_some_and(|(s, e)| i >= s && i < e) {
                // Selected characters are highlighted (Tokyo Night selection
                // blue) over the field's own colours.
                (
                    Some(Color::White),
                    parse_color_str(SELECTION_BG),
                )
            } else {
                (fg, bg)
            };
            // ASCII glyphs write one cell. The second column of a wide CJK
            // glyph is marked with an *empty* word (not a space) so the
            // renderer skips it and the terminal prints the glyph across its
            // two real columns — space cells here would double-print and shift
            // every following column. fg / bg / style stay on the pad cell so
            // the background reads as continuous.
            put(
                canvas,
                cell_index(canvas, abs_col, r.y),
                if sub == 0 { Some(ch) } else { None },
                cell_fg,
                cell_bg,
            );
        }
        col += w;
    }

    // Caret past the last character: an inverted blank cell at the content
    // end (kept inside the frame).
    if focused && cursor == total && col < inner_width {
        let abs_col = inner_x.saturating_add(col);
        if abs_col < r.x.saturating_add(r.width).saturating_sub(1) {
            put(
                canvas,
                cell_index(canvas, abs_col, r.y),
                Some(' '),
                Some(bg.unwrap_or(Color::Black)),
                Some(fg.unwrap_or(Color::White)),
            );
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
