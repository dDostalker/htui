//! Input-field painting — a background-filled single-line field (no bracket
//! frame) with an optional caret cell and a focused frame colour. A `border:`
//! declaration opts into a CSS frame; otherwise the field reads as a plain
//! rectangle, like a browser's default `<input>`.

use compact_str::CompactString;
use crossterm::style::Color;

use crate::{
    graphics::{FontStyle, char_width},
    layout::{LayoutBox, Rect},
    tdom::node::{Node, NodeArena},
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

/// Numeric parameters for an `<input type=range>` control.
///
/// The terminal slider shares this model between painting and interaction so
/// `min`, `max`, `step`, thumb geometry, and pointer mapping cannot drift.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RangeSpec {
    min: f64,
    max: f64,
    step: f64,
    value: f64,
    precision: usize,
}

impl RangeSpec {
    /// Snap the value to the nearest step, then clamp it to `[min, max]`.
    fn normalized(mut self) -> Self {
        self.max = self.max.max(self.min);
        self.step = if self.step > 0.0 { self.step } else { 1.0 };
        self.value = self.min + ((self.value - self.min) / self.step).round() * self.step;
        self.value = self.value.clamp(self.min, self.max);
        self
    }

    /// Canonical DOM representation: integral steps stay integers while
    /// fractional steps use the input scale, avoiding binary-float artifacts.
    pub(crate) fn value_string(&self) -> CompactString {
        if self.step.abs() >= 1.0 && (self.value - self.value.round()).abs() < f64::EPSILON {
            CompactString::from(format!("{}", self.value.round() as i64))
        } else if self.precision == 0 {
            CompactString::from(format!("{}", self.value))
        } else {
            let mut value = format!("{:.*}", self.precision.min(15), self.value);
            if value.contains('.') {
                value = value
                    .trim_end_matches('0')
                    .trim_end_matches('.')
                    .to_string();
            }
            CompactString::from(value)
        }
    }

    /// Thumb position along a track of `length` cells (zero-based; a one-cell
    /// track contains only the thumb).
    pub(crate) fn thumb_at(&self, length: usize) -> usize {
        if length <= 1 {
            return 0;
        }
        let span = self.max - self.min;
        let fraction = if span > 0.0 {
            ((self.value - self.min) / span).clamp(0.0, 1.0)
        } else {
            0.0
        };
        ((fraction * (length - 1) as f64).round() as isize).clamp(0, (length - 1) as isize) as usize
    }

    /// Value for a thumb position, using the same rounding as
    /// [`Self::thumb_at`].
    pub(crate) fn value_at(&self, position: usize, length: usize) -> Self {
        let mut snapped = *self;
        if length <= 1 {
            snapped.value = self.min;
            return snapped.normalized();
        }
        let position = position.min(length - 1);
        let span = self.max - self.min;
        snapped.value = self.min
            + if span > 0.0 {
                position as f64 / (length - 1) as f64 * span
            } else {
                0.0
            };
        snapped.normalized()
    }

    /// Move by whole steps in `direction`; the boundary flags support Home and
    /// End. Returns `false` when the resulting value is unchanged.
    pub(crate) fn advance(&mut self, direction: isize, to_min: bool, to_max: bool) -> bool {
        let previous = self.value;
        if to_min {
            self.value = self.min;
        } else if to_max {
            self.value = self.max;
        } else if direction != 0 {
            self.value += direction as f64 * self.step;
        } else {
            return false;
        }
        let next = self.normalized();
        let changed = next.value != previous;
        *self = next;
        changed
    }
}

fn finite_number(value: &str) -> Option<f64> {
    value.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// `true` for a Firefox-style vertical slider (`<input type=range orient=vertical>`).
pub(crate) fn range_is_vertical(node: &Node) -> bool {
    node.attributes
        .get("orient")
        .is_some_and(|v| v.eq_ignore_ascii_case("vertical"))
}

fn fractional_digits(value: &str) -> usize {
    value
        .trim()
        .rsplit_once('.')
        .map(|(_, fraction)| fraction.bytes().take_while(u8::is_ascii_digit).count())
        .unwrap_or_default()
}

/// Build slider constraints from attributes. `current` is the focused edit
/// value; otherwise the DOM `value` is used, and an invalid or absent value
/// falls back to the browser midpoint default.
pub(crate) fn range_spec(node: &Node, current: Option<&str>) -> RangeSpec {
    let attr = |name: &str| node.attributes.get(name).and_then(|v| finite_number(v));
    let attr_digits = |name: &str| {
        node.attributes
            .get(name)
            .map(|v| fractional_digits(v))
            .unwrap_or_default()
    };
    let min = attr("min").unwrap_or(0.0);
    let max_default = if min > 100.0 { min } else { 100.0 };
    let max = attr("max").unwrap_or(max_default);
    let value = current
        .and_then(finite_number)
        .or_else(|| attr("value"))
        .unwrap_or((min + max) / 2.0);
    RangeSpec {
        min,
        max,
        step: attr("step").unwrap_or(1.0),
        value,
        precision: attr_digits("min").max(attr_digits("step")),
    }
    .normalized()
}

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
    // Control chrome and replaced glyphs are painted directly by cell; reduce
    // the control rect to the inherited clip so a partially visible field
    // cannot overwrite a parent border or sibling cell.
    let r = visible;

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

    // `<input type=range>` paints a one-row horizontal slider. The painter
    // and pointer mapping share `RangeSpec`, so the thumb column, keyboard
    // steps, and click positions agree.
    if input_type == "range" {
        let content = paint_control_chrome(lb, arena, canvas, r, fg);
        let state = canvas.input_states.get(&lb.node_id);
        let spec = range_spec(node, state.map(|s| s.value.as_str()));
        let disabled = node.attributes.contains_key("disabled");
        let track_fg = if disabled {
            Some(Color::DarkGrey)
        } else {
            Some(fg.unwrap_or(Color::White))
        };

        if range_is_vertical(node) {
            if content.height == 0 {
                return;
            }
            let track_x = content.x;
            let thumb_y = content
                .y
                .saturating_add(spec.thumb_at(content.height.max(1)));
            for row in 0..content.height.max(1) {
                let abs_row = content.y.saturating_add(row);
                let (glyph, color) = if abs_row == thumb_y {
                    ('\u{25CF}', track_fg)
                } else {
                    ('\u{2502}', Some(Color::DarkGrey))
                };
                put(
                    canvas,
                    cell_index(canvas, track_x, abs_row),
                    Some(glyph),
                    color,
                    None,
                );
            }
            return;
        }

        if content.width == 0 {
            return;
        }
        let thumb_x = content.x.saturating_add(spec.thumb_at(content.width));
        let track_y = content.y;
        for col in 0..content.width {
            let abs_col = content.x.saturating_add(col);
            let (glyph, color) = if abs_col == thumb_x {
                ('\u{25CF}', track_fg)
            } else if abs_col < thumb_x {
                ('\u{2501}', track_fg)
            } else {
                ('\u{2500}', Some(Color::DarkGrey))
            };
            put(
                canvas,
                cell_index(canvas, abs_col, track_y),
                Some(glyph),
                color,
                None,
            );
        }
        return;
    }

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
            node.attributes
                .get("value")
                .cloned()
                .map(CompactString::from)
                .unwrap_or_default(),
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

#[cfg(test)]
mod range_tests {
    use super::*;
    use crate::tdom::node::NodeType;

    fn range_node(min: &str, max: &str, step: &str, value: &str) -> Node {
        let mut node = Node::from(NodeType::Input);
        for (name, value) in [("min", min), ("max", max), ("step", step), ("value", value)] {
            node.attributes.insert(name.to_string(), value.to_string());
        }
        node
    }

    fn range_vertical_node() -> Node {
        let mut node = range_node("0", "4", "1", "2");
        node.attributes
            .insert("orient".to_string(), "vertical".to_string());
        node
    }

    #[test]
    fn range_thumb_and_pointer_use_the_same_track_mapping() {
        let node = range_node("0", "4", "1", "2");
        let spec = range_spec(&node, None);
        assert_eq!(spec.thumb_at(9), 4);
        assert_eq!(spec.value_at(8, 9).value_string(), "4");
        assert_eq!(spec.value_at(0, 9).value_string(), "0");
    }

    #[test]
    fn range_vertical_mapping_uses_track_rows() {
        assert!(range_is_vertical(&range_vertical_node()));
        let spec = range_spec(&range_vertical_node(), None);
        assert_eq!(spec.thumb_at(5), 2);
        assert_eq!(spec.value_at(0, 5).value_string(), "0");
        assert_eq!(spec.value_at(4, 5).value_string(), "4");
    }

    #[test]
    fn range_keyboard_steps_snap_and_clamp() {
        let mut spec = range_spec(&range_node("0", "4", "1", "0"), None);
        assert!(
            !spec.advance(0, true, false),
            "Home at the minimum is a no-op"
        );
        assert!(spec.advance(2, false, false));
        assert_eq!(spec.value_string(), "2");
        assert!(spec.advance(0, false, true));
        assert_eq!(spec.value_string(), "4");
        assert!(
            !spec.advance(1, false, false),
            "Right at the maximum is a no-op"
        );
    }

    #[test]
    fn range_fractional_steps_use_the_snapped_decimal() {
        let mut spec = range_spec(&range_node("0", "1", "0.25", "0.13"), None);
        assert_eq!(spec.value_string(), "0.25");
        assert!(spec.advance(2, false, false));
        assert_eq!(spec.value_string(), "0.75");
        let mut spec = range_spec(&range_node("0", "3", "0.3", "0.15"), None);
        assert!(spec.advance(2, false, false));
        assert_eq!(spec.value_string(), "0.9");
    }
}
