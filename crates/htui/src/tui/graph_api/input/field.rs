use std::time::Duration;

use compact_str::CompactString;
use crossterm::event::KeyCode;

use crate::events::HitMap;
use crate::layout::Rect;
use crate::layout::paint::{
    range_is_vertical, range_spec, textarea_caret_position, textarea_char_at, textarea_scroll_row,
    textarea_visual_lines, value_window,
};
use crate::tdom::node::{NodeArena, NodeId, NodeType};
use crate::tui::graph_api::char_width;

use super::InputState;
use super::cmd::RenderCmd;
use super::edit::{apply_input_key, char_to_byte};

/// A page-level built-in action deferred pending its `keydown` gate verdict.
///
/// When the script layer is active, scroll / zoom / quit keys are dispatched
/// to JS first; if a listener calls `event.preventDefault()` the deferred
/// commands are dropped (the page owns that key), otherwise the input loop
/// applies them. The wait is bounded by [`KEY_GATE_TIMEOUT`], so a busy or
/// blocked script thread falls back to running the built-in action anyway.
#[derive(Debug)]
pub struct KeyGate {
    /// Key id echoed by the script thread's `KeyDown` response (matching).
    pub id: u32,
    /// The built-in commands to apply when the gate clears.
    pub cmds: Vec<RenderCmd>,
}

/// How long the input loop waits for the script thread's `keydown` verdict
/// before falling back to the built-in shortcut. Sub-millisecond when the
/// script thread is healthy; the bound keeps scrolling / quitting responsive
/// even while a host function blocks the script thread.
pub const KEY_GATE_TIMEOUT: Duration = Duration::from_millis(8);

/// Apply one text-editing key to an input field's value and caret.
///
/// `cursor` is a character index into `value`. Returns `true` when the value
/// or caret changed (the caller should repaint the field); `false` for keys
/// the field does not consume (`Esc` / `Enter` are handled by the caller).
pub fn apply_textarea_key(
    value: &mut CompactString,
    cursor: &mut usize,
    content_width: usize,
    key: KeyCode,
) -> bool {
    match key {
        KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
            let byte = char_to_byte(value, *cursor);
            value.insert(byte, '\n');
            *cursor = cursor.saturating_add(1);
            true
        }
        KeyCode::Up => textarea_move_vertical(value, cursor, content_width, false),
        KeyCode::Down => textarea_move_vertical(value, cursor, content_width, true),
        _ => apply_input_key(value, cursor, key),
    }
}

/// Move the caret one visual line up / down, keeping the current column
/// (clamped to the target line's length). Returns `false` at the boundaries.
fn textarea_move_vertical(
    value: &str,
    cursor: &mut usize,
    content_width: usize,
    down: bool,
) -> bool {
    let (row, col) = textarea_caret_position(value, content_width, *cursor);
    let total = textarea_visual_lines(value, content_width).len();
    if down {
        if row.saturating_add(1) >= total {
            return false;
        }
    } else if row == 0 {
        return false;
    }
    let target_row = if down {
        row.saturating_add(1)
    } else {
        row.saturating_sub(1)
    };
    let new_cursor = textarea_char_at(value, content_width, target_row, col);
    if new_cursor == *cursor {
        return false;
    }
    *cursor = new_cursor;
    true
}

/// Content width (cells) of the focused textarea from its `cols` attribute
/// (falling back to the browser default of 20), matching the geometry pass's
/// sizing when the box is not clamped by the containing block.
pub(super) fn textarea_content_width(arena: Option<&NodeArena>, node_id: NodeId) -> usize {
    arena
        .and_then(|a| a.get(node_id))
        .and_then(|n| n.attributes.get("cols"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
        .max(1)
}

/// `true` for an `<input type=checkbox|radio>` node — a click toggles the
/// control instead of focusing a text field.
pub(super) fn is_toggle_input(arena: &NodeArena, node_id: NodeId) -> bool {
    arena.get(node_id).is_some_and(|n| {
        n.get_type()
            .is_some_and(|t| matches!(t.to_ascii_lowercase().as_str(), "checkbox" | "radio"))
    })
}

/// `true` for an `<input type=button|submit|reset|image>` node — a click
/// activates the push-button (the form submit / reset default action fires on
/// the script thread) instead of focusing a text field.
pub(super) fn is_button_input(arena: &NodeArena, node_id: NodeId) -> bool {
    arena.get(node_id).is_some_and(|n| {
        n.node_type == NodeType::Input
            && n.get_type().is_some_and(|t| {
                matches!(
                    t.to_ascii_lowercase().as_str(),
                    "button" | "submit" | "reset" | "image"
                )
            })
    })
}

/// `true` for an `<input type=number>` node — plain typing is filtered to the
/// numeric character set the browser's number editor accepts.
pub(super) fn is_number_input(arena: Option<&NodeArena>, node_id: NodeId) -> bool {
    arena.is_some_and(|a| {
        a.get(node_id).is_some_and(|n| {
            n.get_type()
                .is_some_and(|t| t.eq_ignore_ascii_case("number"))
        })
    })
}

/// `true` for `<input type=range>` — a discrete slider, not a text field.
pub(super) fn is_range_input(arena: &NodeArena, node_id: NodeId) -> bool {
    arena.get(node_id).is_some_and(|n| {
        n.get_type()
            .is_some_and(|t| t.eq_ignore_ascii_case("range"))
    })
}

/// Normalize the focused slider's edit buffer to its current constraints.
pub(super) fn normalize_range_state(
    state: &mut InputState,
    arena: &NodeArena,
    node_id: NodeId,
) -> bool {
    let Some(node) = arena.get(node_id) else {
        return false;
    };
    let spec = range_spec(node, Some(state.input_value.as_str()));
    let value = spec.value_string();
    let changed = state.input_value != value;
    state.input_value = value;
    state.input_cursor = state.input_value.chars().count();
    changed
}

/// Apply a keyboard adjustment to a focused range. Horizontal sliders use
/// Left/Down then Right/Up; vertical sliders invert Up/Down so the keys follow
/// track direction. Home/End jump to the boundaries.
pub(super) fn apply_range_key(
    state: &mut InputState,
    arena: &NodeArena,
    node_id: NodeId,
    key: KeyCode,
) -> bool {
    let Some(node) = arena.get(node_id) else {
        return false;
    };
    let mut spec = range_spec(node, Some(state.input_value.as_str()));
    let vertical = range_is_vertical(node);
    let changed = match key {
        KeyCode::Left => spec.advance(-1, false, false),
        KeyCode::Down if vertical => spec.advance(1, false, false),
        KeyCode::Down => spec.advance(-1, false, false),
        KeyCode::Right => spec.advance(1, false, false),
        KeyCode::Up if vertical => spec.advance(-1, false, false),
        KeyCode::Up => spec.advance(1, false, false),
        KeyCode::Home => spec.advance(0, true, false),
        KeyCode::End => spec.advance(0, false, true),
        _ => false,
    };
    if changed {
        state.input_value = spec.value_string();
        state.input_cursor = state.input_value.chars().count();
    }
    changed
}

/// Set a range from a pointer column over its laid-out rectangle. The helper
/// uses the same track mapping as the painter so clicks and drags land on the
/// thumb position the user sees.
pub(super) fn set_range_at_pointer(
    state: &mut InputState,
    arena: &NodeArena,
    node_id: NodeId,
    rect: crate::layout::Rect,
    x: u32,
    y: u32,
) -> bool {
    let Some(node) = arena.get(node_id) else {
        return false;
    };
    let spec = range_spec(node, Some(state.input_value.as_str()));
    let value = if range_is_vertical(node) {
        let local_y =
            ((y as usize).saturating_sub(rect.y)).min(rect.height.max(1).saturating_sub(1));
        spec.value_at(local_y, rect.height.max(1))
    } else {
        let local_x =
            ((x as usize).saturating_sub(rect.x)).min(rect.width.max(1).saturating_sub(1));
        spec.value_at(local_x, rect.width.max(1))
    }
    .value_string();
    let changed = state.input_value != value;
    state.input_value = value;
    state.input_cursor = state.input_value.chars().count();
    changed
}

/// The character set an `<input type=number>` field accepts while typing:
/// digits plus the sign / decimal / exponent markers the browser's number
/// editor recognises (`-1.5e+3`).
pub fn number_input_allows_char(c: char) -> bool {
    c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E')
}

/// `true` for a `<input readonly>` text field — it can be focused, selected
/// and copied like any other field, but no keystroke or paste may change its
/// value.
pub(super) fn is_readonly_input(arena: Option<&NodeArena>, node_id: NodeId) -> bool {
    arena.is_some_and(|a| {
        a.get(node_id).is_some_and(|n| {
            n.node_type == NodeType::Input && n.attributes.contains_key("readonly")
        })
    })
}

/// `true` for a form control carrying a `disabled` attribute — such controls
/// swallow clicks and presses (no Click dispatch, no focus change, no
/// dropdown) and are skipped by `Tab` focus cycling, matching browsers.
pub(super) fn is_disabled_control(arena: Option<&NodeArena>, node_id: NodeId) -> bool {
    arena.is_some_and(|a| {
        a.get(node_id).is_some_and(|n| {
            n.attributes.contains_key("disabled")
                && matches!(
                    n.node_type,
                    NodeType::Button | NodeType::Input | NodeType::Select | NodeType::Textarea
                )
        })
    })
}

/// The nearest `<form>` ancestor of `node_id`, if any — the target of the
/// browser's implicit submission when Enter is pressed in a text field.
pub(super) fn focused_form(arena: Option<&NodeArena>, node_id: NodeId) -> Option<NodeId> {
    let arena = arena?;
    let mut current = arena.get(node_id).and_then(|n| n.parent);
    while let Some(pid) = current {
        if arena
            .get(pid)
            .is_some_and(|n| n.node_type == NodeType::Form)
        {
            return Some(pid);
        }
        current = arena.get(pid).and_then(|n| n.parent);
    }
    None
}

/// `true` for a `<textarea>` node — the multi-line field whose caret
/// placement uses visual row/col, not the single-line column mapping.
pub(super) fn is_textarea_node(arena: &NodeArena, node_id: NodeId) -> bool {
    arena
        .get(node_id)
        .is_some_and(|n| n.node_type == NodeType::Textarea)
}

/// Map a pointer cell inside a `<textarea>` rect to a character index, using
/// the same wrapped-line + caret-derived scroll the painter draws. The rect
/// is the whole field — with an opt-in CSS border the one-cell frame offsets
/// are ignored for hit-mapping (a frame cell click maps to the first/last
/// content cell).
pub(super) fn textarea_index_at(state: &InputState, rect: Rect, x: u32, y: u32) -> usize {
    let content_width = rect.width.max(1);
    let rows = rect.height.max(1);
    let scroll = textarea_scroll_row(&state.input_value, content_width, rows, state.input_cursor);
    let local_row = (y as usize).saturating_sub(rect.y).saturating_add(scroll);
    let local_col = (x as usize).saturating_sub(rect.x);
    textarea_char_at(&state.input_value, content_width, local_row, local_col)
}

/// `true` for a focusable text field (`<input>` / `<textarea>`).
pub(super) fn is_text_field(arena: &NodeArena, node_id: NodeId) -> bool {
    arena
        .get(node_id)
        .is_some_and(|n| matches!(n.node_type, NodeType::Input | NodeType::Textarea))
}

/// `true` when the hit target (or one of its ancestors) owns non-flow control
/// chrome rather than normal page text, or its cascaded CSS resolves
/// `user-select: none`.
///
/// A drag that starts on one of these elements belongs to the control (or is
/// pure chrome); it must not arm the engine's page flow-selection. This is
/// deliberately broader than focusability: button inputs and replaced bars are
/// not text-selection origins even though a plain `<output>` remains normal
/// selectable content.
///
/// The `user-select` verdict comes from the hit map because it is built from
/// the *styled* geometry; the `arena` here is the input loop's raw engine
/// snapshot, which carries no `css_attr`.
pub(super) fn blocks_page_selection(hit_map: &HitMap, arena: &NodeArena, node_id: NodeId) -> bool {
    let mut current = Some(node_id);
    while let Some(id) = current {
        let Some(node) = arena.get(id) else {
            return false;
        };
        if matches!(
            node.node_type,
            NodeType::Input
                | NodeType::Textarea
                | NodeType::Select
                | NodeType::Button
                | NodeType::Img
                | NodeType::Progress
                | NodeType::Meter
        ) {
            return true;
        }
        current = node.parent;
    }
    hit_map.is_unselectable(node_id)
}

/// All `<input>` / `<textarea>` node ids in document order (depth-first), for
/// `Tab` / `Shift`+`Tab` focus cycling. Reads the input loop's DOM snapshot; a
/// removed or not-yet-rendered field is simply not included. `hidden` inputs
/// and checkbox/radio toggles (click-only in this engine) are skipped.
pub fn collect_input_ids(arena: &NodeArena) -> Vec<NodeId> {
    let mut out = Vec::new();
    let Some(root) = arena
        .iter()
        .find(|(_, n)| n.parent.is_none())
        .map(|(id, _)| id)
    else {
        return out;
    };
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let node = &arena[id];
        // A disabled control is unfocusable: browsers skip it in `Tab`
        // cycling entirely. (Only form controls honour `disabled`.)
        if node.attributes.contains_key("disabled")
            && matches!(
                node.node_type,
                NodeType::Button | NodeType::Input | NodeType::Select | NodeType::Textarea
            )
        {
            continue;
        }
        match node.node_type {
            NodeType::Textarea => out.push(id),
            NodeType::Input => {
                let t = node
                    .get_type()
                    .map(|t| t.to_ascii_lowercase())
                    .unwrap_or_default();
                // Button inputs activate on click only — they take no text
                // focus, so `Tab` cycling skips them.
                let is_button = matches!(t.as_str(), "button" | "submit" | "reset" | "image");
                if t != "hidden" && t != "checkbox" && t != "radio" && !is_button {
                    out.push(id);
                }
            }
            _ => {}
        }
        for child in node.children.iter().rev() {
            stack.push(*child);
        }
    }
    out
}

/// Map a field-inner column offset (0-based, relative to the first content
/// column after `[`) to a character index, using the same caret-centred
/// window the painter draws. Pointer positions outside the visible window
/// clamp to the nearest reachable character so the caret never leaves the
/// visible slice.
pub(super) fn char_index_at_x(value: &str, cursor: usize, inner_width: usize, x: usize) -> usize {
    let (start, end) = value_window(value, cursor, inner_width);
    let v: Vec<char> = value.chars().collect();
    let mut col = 0usize;
    for (i, &ch) in v.iter().enumerate().take(end).skip(start) {
        let w = char_width(ch).max(1);
        if x < col + w {
            return i;
        }
        col += w;
    }
    // Past the visible window's right edge: the window's end is the nearest
    // reachable caret (the caret itself is always inside the window).
    if end < v.len() { end } else { v.len() }
}

/// Clear the page text selection, queueing a repaint only when one existed.
/// Returns whether a visible selection was removed (the caller can notify the
/// script layer with `selectionchange`).
pub(super) fn clear_text_selection(state: &mut InputState, cmds: &mut Vec<RenderCmd>) -> bool {
    state.text_sel_armed = false;
    let had_selection = state.text_sel_anchor.is_some() || state.text_sel_end.is_some();
    if had_selection {
        state.text_sel_anchor = None;
        state.text_sel_end = None;
        cmds.push(RenderCmd::TextSelection {
            anchor: None,
            end: None,
            mode: state.text_sel_mode,
        });
    }
    had_selection
}

#[cfg(test)]
mod range_key_tests {
    use super::*;
    use crate::tdom::node::NodeType;

    #[test]
    fn range_keys_step_and_clamp_without_text_editing() {
        let mut arena = NodeArena::default();
        let mut node = crate::tdom::node::Node::from(NodeType::Input);
        node.attributes
            .insert("type".to_string(), "range".to_string());
        node.attributes.insert("min".to_string(), "0".to_string());
        node.attributes.insert("max".to_string(), "2".to_string());
        node.attributes.insert("step".to_string(), "1".to_string());
        node.attributes.insert("value".to_string(), "1".to_string());
        let id = arena.insert(node);

        let mut state = InputState::new();
        state.input_value = CompactString::from("1");
        assert!(apply_range_key(&mut state, &arena, id, KeyCode::Left));
        assert_eq!(state.input_value, "0");
        assert!(!apply_range_key(&mut state, &arena, id, KeyCode::Left));
        assert_eq!(state.input_value, "0");
        assert!(apply_range_key(&mut state, &arena, id, KeyCode::Right));
        assert_eq!(state.input_value, "1");
        assert!(!apply_range_key(&mut state, &arena, id, KeyCode::Char('x')));
        assert_eq!(state.input_value, "1");
    }

    #[test]
    fn vertical_range_keys_follow_the_track() {
        let mut arena = NodeArena::default();
        let mut node = crate::tdom::node::Node::from(NodeType::Input);
        node.attributes
            .insert("type".to_string(), "range".to_string());
        node.attributes
            .insert("orient".to_string(), "vertical".to_string());
        node.attributes.insert("min".to_string(), "0".to_string());
        node.attributes.insert("max".to_string(), "2".to_string());
        node.attributes.insert("value".to_string(), "1".to_string());
        let id = arena.insert(node);

        let mut state = InputState::new();
        state.input_value = CompactString::from("1");
        assert!(apply_range_key(&mut state, &arena, id, KeyCode::Up));
        assert_eq!(state.input_value, "0");
        assert!(apply_range_key(&mut state, &arena, id, KeyCode::Down));
        assert_eq!(state.input_value, "1");
    }
}
