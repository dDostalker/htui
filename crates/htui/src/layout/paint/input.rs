//! Input-field painting — a background-filled single-line field (no bracket
//! frame) with an optional caret cell and a focused frame colour. A `border:`
//! declaration opts into a CSS frame; otherwise the field reads as a plain
//! rectangle, like a browser's default `<input>`.

use crossterm::style::Color;

use crate::{
    graphics::{FontStyle, char_width},
    layout::{LayoutBox, Rect},
    tdom::node::NodeArena,
};

use super::button::paint_button;
use super::{
    PaintCanvas, cell_index, clamp_rect_to_canvas, intersect_rects, paint_control_chrome, put,
};

/// Highlight colour for the selected character range (Tokyo Night selection).
const SELECTION_BG: Color = Color::Rgb {
    r: 0x3b,
    g: 0x42,
    b: 0x61,
};

/// Visible window of `value` for a field with `inner_width` content columns,
/// centred on the caret (the window always contains the caret, extending left
/// then right). Shared by the painter — draw exactly the visible slice — and
/// the input loop — map a pointer column back to a character index — so both
/// must agree on which characters are visible.
pub(crate) fn value_window(value: &str, caret: usize, inner_width: usize) -> (usize, usize) {
    let v: Vec<char> = value.chars().collect();
    let total = v.len();
    let caret = caret.min(total);
    let widths: Vec<usize> = v.iter().map(|&c| char_width(c).max(1)).collect();
    let caret_w: usize = widths.get(caret).copied().unwrap_or(1);

    let mut start = caret;
    let mut before_w = 0usize;
    while start > 0
        && before_w
            .saturating_add(widths[start - 1])
            .saturating_add(caret_w)
            <= inner_width
    {
        before_w += widths[start - 1];
        start -= 1;
    }

    let mut end = caret.saturating_add(1).min(total);
    let mut used = before_w.saturating_add(caret_w);
    while end < total && used.saturating_add(widths[end]) <= inner_width {
        used += widths[end];
        end += 1;
    }
    (start, end)
}

pub(crate) fn paint_input(
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
    let input_type = node
        .get_type()
        .map(|t| t.to_ascii_lowercase())
        .unwrap_or_default();

    // `<input type=button|submit|reset|image>` paints as a push-button sharing
    // the `<button>` painter (the label comes from the `value` attribute, with
    // browser defaults when absent).
    if node.input_button_label().is_some() {
        paint_button(lb, arena, canvas, clip);
        return;
    }

    let fg = node.css_attr.get_color();

    // `<input type=checkbox|radio>` is a bare terminal toggle — no background
    // fill and no frame. Both controls follow the hollow → solid pattern so
    // they read as one family: checkbox `□` / `■`, radio `○` / `●`.
    if matches!(input_type.as_str(), "checkbox" | "radio") {
        let checked = node.attributes.contains_key("checked");
        let disabled = node.attributes.contains_key("disabled");
        let glyph = match (input_type.as_str(), checked) {
            ("checkbox", true) => '\u{25A0}',  // ■
            ("checkbox", false) => '\u{25A1}', // □
            ("radio", true) => '\u{25CF}',     // ●
            _ => '\u{25CB}',                   // ○
        };
        let toggle_fg = if disabled {
            Color::DarkGrey
        } else {
            fg.unwrap_or(Color::White)
        };
        // The glyph carries no background of its own: the surrounding page
        // background shows through, like a browser checkbox.
        put_bare(
            canvas,
            cell_index(canvas, r.x, r.y),
            Some(glyph),
            Some(toggle_fg),
        );
        // One spacing column so the toggle never glues onto adjacent text.
        put_bare(
            canvas,
            cell_index(canvas, r.x.saturating_add(1), r.y),
            Some(' '),
            Some(toggle_fg),
        );
        return;
    }

    let content = paint_control_chrome(lb, arena, canvas, r, fg);
    let bg = node
        .css_attr
        .get_bg_color()
        .or(Some(super::CONTROL_DEFAULT_BG));

    // Live edit state wins while focused, and an unfocused entry covers only
    // the single repaint right after a blur (so the committed text doesn't
    // flash back to the pre-commit DOM value). Every fresh layout drops
    // unfocused entries (see `TermLayout::layout_only`), so an unfocused
    // field otherwise renders from the DOM `value` attribute — the display
    // source of truth.
    let (value, cursor, focused, selection) = match canvas.input_states.get(&lb.node_id) {
        Some(st) => (st.value.clone(), st.cursor, st.focused, st.selection),
        None => (
            node.attributes.get("value").cloned().unwrap_or_default(),
            0,
            false,
            None,
        ),
    };

    // A `placeholder` attribute shows as dimmed hint text while the field is
    // empty and unfocused (browsers hide it once the field has a value or
    // focus).
    let placeholder = node.attributes.get("placeholder").cloned();
    let placeholder = placeholder.as_deref().filter(|p| !p.is_empty());
    let showing_placeholder = value.is_empty() && !focused && placeholder.is_some();
    let display: &str = match (showing_placeholder, placeholder) {
        (true, Some(p)) => p,
        _ => value.as_str(),
    };

    let disabled = node.attributes.contains_key("disabled");
    // A disabled field is dimmed (text grey); the background keeps its fill.
    let text_fg = if disabled { Some(Color::DarkGrey) } else { fg };
    // `caret-color` paints the caret block with the declared colour; the
    // default is the inverted cell (field background <-> text colour swap).
    let caret_color = node.css_attr.get_caret_color();
    let caret_cell = |fg: Option<Color>, bg: Option<Color>| match caret_color {
        Some(cc) => (Some(bg.unwrap_or(Color::Black)), Some(cc)),
        None => (
            Some(bg.unwrap_or(Color::Black)),
            Some(fg.unwrap_or(Color::White)),
        ),
    };

    // The value fills the whole content area.
    let inner_width = content.width;
    let inner_x = content.x;
    let inner_y = content.y;

    let v: Vec<char> = display.chars().collect();
    let cursor = cursor.min(value.chars().count());
    let (start, end) = value_window(display, cursor, inner_width);
    // `<input type=password>` masks every character with `•` while keeping
    // each glyph's cell width (so the caret window / column mapping shared
    // with the input loop stays in sync).
    let is_password = input_type == "password" && !showing_placeholder;

    let mut col = 0usize;
    for (i, &ch) in v.iter().enumerate().take(end).skip(start) {
        let w = char_width(ch).max(1);
        let glyph = if is_password { '\u{2022}' } else { ch };
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
                caret_cell(fg, bg)
            } else if selection.is_some_and(|(s, e)| i >= s && i < e) {
                // Selected characters are highlighted (Tokyo Night selection
                // blue) over the field's own colours.
                (Some(Color::White), Some(SELECTION_BG))
            } else if showing_placeholder {
                (Some(Color::DarkGrey), bg)
            } else {
                (text_fg, bg)
            };
            // ASCII glyphs write one cell. The second column of a wide CJK
            // glyph is marked with an *empty* word (not a space) so the
            // renderer skips it and the terminal prints the glyph across its
            // two real columns — space cells here would double-print and shift
            // every following column. fg / bg / style stay on the pad cell so
            // the background reads as continuous.
            put(
                canvas,
                cell_index(canvas, abs_col, inner_y),
                if sub == 0 { Some(glyph) } else { None },
                cell_fg,
                cell_bg,
            );
        }
        col += w;
    }

    // Caret past the last character: an inverted blank cell at the content
    // end (kept inside the field).
    if focused && cursor == value.chars().count() && col < inner_width {
        let abs_col = inner_x.saturating_add(col);
        if abs_col < inner_x.saturating_add(inner_width) {
            let (cf, cb) = caret_cell(fg, bg);
            put(
                canvas,
                cell_index(canvas, abs_col, inner_y),
                Some(' '),
                cf,
                cb,
            );
        }
    }
}

/// Write a toggle cell: word + fg only — the background stays whatever the
/// surrounding page painted, so a checkbox / radio reads as a bare glyph on
/// the page background instead of a filled field.
fn put_bare(canvas: &mut PaintCanvas<'_>, idx: usize, word: Option<char>, fg: Option<Color>) {
    if idx >= canvas.layout.len() || !canvas.in_region(idx) {
        return;
    }
    let cell = &mut canvas.layout[idx];
    cell.word.clear();
    if let Some(c) = word {
        cell.word.push(c);
    }
    cell.fcolor = fg;
    cell.style = FontStyle::empty();
}
