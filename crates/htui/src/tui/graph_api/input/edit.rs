use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::tdom::node::NodeId;

use super::InputState;
use super::cmd::RenderCmd;
use super::focus::push_input_render;

/// `cursor` is a character index into `value`. Returns `true` when the value
/// or caret changed (the caller should repaint the field); `false` for keys
/// the field does not consume (`Esc` / `Enter` are handled by the caller).
pub fn apply_input_key(value: &mut String, cursor: &mut usize, key: KeyCode) -> bool {
    match key {
        KeyCode::Char(c) => {
            if c == '\r' || c == '\n' {
                return false;
            }
            let byte = char_to_byte(value, *cursor);
            value.insert(byte, c);
            *cursor = cursor.saturating_add(1);
            true
        }
        KeyCode::Backspace => {
            if *cursor == 0 {
                return false;
            }
            let new_cursor = cursor.saturating_sub(1);
            let byte = char_to_byte(value, new_cursor);
            value.remove(byte);
            *cursor = new_cursor;
            true
        }
        KeyCode::Delete => {
            let len = value.chars().count();
            if *cursor >= len {
                return false;
            }
            let byte = char_to_byte(value, *cursor);
            value.remove(byte);
            true
        }
        KeyCode::Left => {
            if *cursor > 0 {
                *cursor -= 1;
                true
            } else {
                false
            }
        }
        KeyCode::Right => {
            if *cursor < value.chars().count() {
                *cursor += 1;
                true
            } else {
                false
            }
        }
        KeyCode::Home => {
            if *cursor != 0 {
                *cursor = 0;
                true
            } else {
                false
            }
        }
        KeyCode::End => {
            let len = value.chars().count();
            if *cursor != len {
                *cursor = len;
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Keys a focused `<input>` field owns. Editing / caret keys stay inside the
/// field — even when they are boundary no-ops (Home at the start) — while
/// everything else (page navigation like ↓ / ↑ / PageDown / PageUp, function
/// keys) belongs to the page-level handler.
pub fn is_input_field_key(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::Char(_)
            | KeyCode::Backspace
            | KeyCode::Delete
            | KeyCode::Left
            | KeyCode::Right
            | KeyCode::Home
            | KeyCode::End
    )
}

/// Character index of the previous word boundary at or before `cursor`.
///
/// Whitespace runs are skipped, then one word (alphanumeric / underscore) or
/// one run of other non-space characters — browser / vim `b` semantics. Used
/// by `Ctrl`/`Alt`+`←` word navigation and `Ctrl`+`Backspace`.
pub fn prev_word_boundary(value: &str, cursor: usize) -> usize {
    let chars: Vec<char> = value.chars().collect();
    let mut i = cursor.min(chars.len());
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    if i > 0 && is_word_char(chars[i - 1]) {
        while i > 0 && is_word_char(chars[i - 1]) {
            i -= 1;
        }
    } else {
        while i > 0 && !is_word_char(chars[i - 1]) && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
    }
    i
}

/// Character index of the next word boundary at or after `cursor` (browser /
/// vim `w` semantics).
pub fn next_word_boundary(value: &str, cursor: usize) -> usize {
    let chars: Vec<char> = value.chars().collect();
    let mut i = cursor.min(chars.len());
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    if i < chars.len() && is_word_char(chars[i]) {
        while i < chars.len() && is_word_char(chars[i]) {
            i += 1;
        }
    } else {
        while i < chars.len() && !is_word_char(chars[i]) && !chars[i].is_whitespace() {
            i += 1;
        }
    }
    i
}

/// A word character: alphanumeric or underscore (`\w` without the locale
/// quirks).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Apply a `Ctrl` / `Alt` word-level editing key to a field's edit state.
///
/// Handles word navigation (`Ctrl`/`Alt`+`←`/`→`, `Ctrl`+`Home`/`End`), word
/// deletion (`Ctrl`+`Backspace` / `Ctrl`+`Delete`) and selection extension
/// (`Shift` combined with the navigation keys). Returns `true` when the value
/// or caret moved (the caller repaints the field); `false` for keys this
/// helper does not own. Non-navigation word edits clear the selection, like
/// plain typing.
pub fn apply_input_word_key(
    value: &mut String,
    cursor: &mut usize,
    sel_anchor: &mut Option<usize>,
    key: KeyCode,
    modifiers: KeyModifiers,
) -> bool {
    let ctrl = modifiers.contains(event::KeyModifiers::CONTROL);
    let alt = modifiers.contains(event::KeyModifiers::ALT);
    let shift = modifiers.contains(event::KeyModifiers::SHIFT);
    if !ctrl && !alt {
        return false;
    }
    let before = *cursor;
    match key {
        KeyCode::Left => {
            *cursor = prev_word_boundary(value, *cursor);
        }
        KeyCode::Right => {
            *cursor = next_word_boundary(value, *cursor);
        }
        KeyCode::Home => *cursor = 0,
        KeyCode::End => *cursor = value.chars().count(),
        KeyCode::Backspace if ctrl => {
            let new_cursor = prev_word_boundary(value, *cursor);
            let sb = char_to_byte(value, new_cursor);
            let eb = char_to_byte(value, *cursor);
            value.drain(sb..eb);
            *cursor = new_cursor;
            *sel_anchor = None;
            return true;
        }
        KeyCode::Delete if ctrl => {
            let end = next_word_boundary(value, *cursor);
            let sb = char_to_byte(value, *cursor);
            let eb = char_to_byte(value, end);
            value.drain(sb..eb);
            *sel_anchor = None;
            return true;
        }
        _ => return false,
    }
    if *cursor != before {
        if shift {
            // Selection extends from the original caret position.
            let anchor = sel_anchor.unwrap_or(before);
            *sel_anchor = Some(anchor);
        } else {
            *sel_anchor = None;
        }
        true
    } else {
        false
    }
}

/// Byte offset of the `char_idx`-th character (clamped to the end).
pub(super) fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(byte, _)| byte)
        .unwrap_or(s.len())
}

/// Ordered `(start, end)` character range between a selection anchor and the
/// caret (`start <= end`).
pub fn selection_range(anchor: usize, cursor: usize) -> (usize, usize) {
    (anchor.min(cursor), anchor.max(cursor))
}

/// The selected substring of `value` for character range `(start, end)`.
fn selected_substring(value: &str, sel: (usize, usize)) -> String {
    let (s, e) = sel;
    let sb = char_to_byte(value, s);
    let eb = char_to_byte(value, e);
    value[sb..eb].to_string()
}

/// Apply a `Ctrl` shortcut to the focused field's edit state.
///
/// `Ctrl+A` selects all, `Ctrl+C` copies the selection to the session
/// clipboard, `Ctrl+X` cuts it, `Ctrl+V` pastes. Returns `true` when the
/// value / caret / selection changed and the field must repaint (copy alone
/// does not).
pub(super) fn apply_ctrl_shortcut(state: &mut InputState, key: KeyCode) -> bool {
    match key {
        KeyCode::Char('a') => {
            let len = state.input_value.chars().count();
            state.input_sel_anchor = Some(0);
            state.input_cursor = len;
            true
        }
        KeyCode::Char('c') => {
            let Some(anchor) = state.input_sel_anchor else {
                return false;
            };
            let (s, e) = selection_range(anchor, state.input_cursor);
            if s == e {
                return false;
            }
            state.input_clipboard = selected_substring(&state.input_value, (s, e));
            false
        }
        KeyCode::Char('x') => {
            let Some(anchor) = state.input_sel_anchor else {
                return false;
            };
            let (s, e) = selection_range(anchor, state.input_cursor);
            if s == e {
                return false;
            }
            state.input_clipboard = selected_substring(&state.input_value, (s, e));
            let sb = char_to_byte(&state.input_value, s);
            let eb = char_to_byte(&state.input_value, e);
            state.input_value.drain(sb..eb);
            state.input_cursor = s;
            state.input_sel_anchor = None;
            true
        }
        KeyCode::Char('v') => {
            let text = state.input_clipboard.clone();
            if text.is_empty() {
                return false;
            }
            // Single-line fields collapse newlines from the internal
            // clipboard (e.g. copied page text) — same rule as bracketed
            // paste; a `<textarea>` keeps them.
            let text = if state.focused_is_textarea {
                text
            } else {
                text.chars()
                    .filter(|c| *c != '\r' && *c != '\n')
                    .collect::<String>()
            };
            if text.is_empty() {
                return false;
            }
            let byte = char_to_byte(&state.input_value, state.input_cursor);
            state.input_value.insert_str(byte, &text);
            state.input_cursor = state.input_cursor.saturating_add(text.chars().count());
            state.input_sel_anchor = None;
            true
        }
        _ => false,
    }
}

/// Extend / shrink the selection with `Shift` + navigation, remembering the
/// anchor when the selection starts. Returns `true` when the caret moved.
pub(super) fn apply_shift_navigation(state: &mut InputState, key: KeyCode) -> bool {
    let total = state.input_value.chars().count();
    let anchor = state.input_sel_anchor.get_or_insert(state.input_cursor);
    let anchor = *anchor;
    let before = state.input_cursor;
    match key {
        KeyCode::Left => {
            if state.input_cursor > 0 {
                state.input_cursor -= 1;
            }
        }
        KeyCode::Right => {
            if state.input_cursor < total {
                state.input_cursor += 1;
            }
        }
        KeyCode::Home => state.input_cursor = 0,
        KeyCode::End => state.input_cursor = total,
        _ => return false,
    }
    state.input_sel_anchor = Some(anchor);
    state.input_cursor != before
}

/// Insert pasted text into the focused field's edit buffer at the caret,
/// collapsing newlines for single-line fields (kept for `<textarea>`), then
/// repaint the field. Returns `true` when the value changed (the caller
/// queues the `input` event). Shared by bracketed paste (`Event::Paste`) and
/// the internal `Ctrl+V`, so both paths behave identically.
pub fn paste_into_field(
    state: &mut InputState,
    text: &str,
    focused: NodeId,
    cmds: &mut Vec<RenderCmd>,
) -> bool {
    let text = if state.focused_is_textarea {
        text.to_string()
    } else {
        text.chars()
            .filter(|c| *c != '\r' && *c != '\n')
            .collect::<String>()
    };
    if text.is_empty() {
        return false;
    }
    let byte = char_to_byte(&state.input_value, state.input_cursor);
    state.input_value.insert_str(byte, &text);
    state.input_cursor = state.input_cursor.saturating_add(text.chars().count());
    state.input_sel_anchor = None;
    push_input_render(state, focused, cmds);
    true
}
