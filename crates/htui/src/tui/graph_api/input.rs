use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};

use crate::{
    script::ScriptMsg,
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::char_width,
};
use crate::events::HitMap;
use crate::layout::layoutbox::geometry::{select_options, select_overlay_rect};
use crate::layout::paint::value_window;

const SCROLL_LINES_PER_TICK: u32 = 3;
const ZOOM_STEP: f32 = 0.1;
const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 5.0;

/// Shape of the page text selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionMode {
    /// Left-drag: continuous text between the anchor and the current point,
    /// spanning whole lines in between (browser-like reading order).
    Flow,
    /// Right-drag: rectangular column selection (vim `Ctrl+V` style).
    Block,
}

/// DOM mutations to repaint on the next `RenderArena` frame.
///
/// The script engine records the session-local node ids whose content or
/// attributes JS changed since the last render. When the set is non-empty the
/// render thread can try a region-limited repaint (re-layout the tree, then
/// repaint only the affected rectangles) instead of clearing and re-painting
/// the whole canvas; the rect comparison against the cached layout decides
/// whether that path is safe and falls back to a full repaint otherwise.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DirtyInfo {
    pub nodes: Vec<crate::tdom::node::NodeId>,
}

impl DirtyInfo {
    /// True when no DOM mutation is pending (nothing to repaint).
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[derive(Debug, Clone)]
pub enum RenderCmd {
    Rerender { scroll_y: u32 },
    Redraw { scroll_y: u32, zoom: f32 },
    /// Render a supplied DOM arena snapshot (the script engine's live arena
    /// after JS mutations), cascading styles and re-laying-out without
    /// re-parsing HTML. `hovered` is the current `:hover` target so the
    /// re-cascade re-applies the highlight in the same pass (no separate
    /// `Hover` command needed). `dirty` lists the nodes JS mutated since the
    /// previous frame, letting the render thread repaint only the affected
    /// rectangles when the re-layout leaves every box rect unchanged.
    RenderArena {
        arena: Arc<crate::tdom::node::NodeArena>,
        root_id: crate::tdom::node::NodeId,
        scroll_y: u32,
        zoom: f32,
        hovered: Option<crate::tdom::node::NodeId>,
        dirty: DirtyInfo,
    },
    /// Replace the session document with `html` and fully re-render it,
    /// keeping the current scroll position / zoom. Script-path uses are
    /// intercepted by `ScriptEngine::reload` so the engine re-parses the
    /// document (single source of truth); the render-thread branch only
    /// handles direct API use / fallback.
    RenderHtml { html: Arc<str> },
    /// Re-style the cached document for the CSS `:hover` pseudo-class with the
    /// given session-local node (`None` clears any hover), then repaint. With
    /// no `:hover` rules in the stylesheet this repaints to the same pixels.
    Hover { node_id: Option<NodeId> },
    /// Repaint one `<input>` field with fresh edit state (value / caret /
    /// focus). The layout is unchanged — the field's rectangle is fixed — so
    /// only that rectangle is repainted.
    RenderInput {
        node_id: NodeId,
        value: String,
        cursor: usize,
        focused: bool,
        selection: Option<(usize, usize)>,
    },
    /// Open the `<select>` dropdown overlay for `node_id`: the render thread
    /// reads the options from the DOM, paints the overlay below the field and
    /// recolours the field's frame.
    SelectOpen { node_id: NodeId },
    /// Close the dropdown overlay for `node_id` (clears the painted overlay
    /// and restores the field's frame).
    SelectClose { node_id: NodeId },
    /// Move the dropdown's highlighted row to `highlight` (keyboard
    /// navigation) and repaint only the overlay.
    SelectMove {
        node_id: NodeId,
        highlight: usize,
    },
    /// Update the page text selection (canvas-space anchor and current
    /// endpoint; either `None` clears it). The render thread repaints the
    /// affected region with the selection highlight (flow or block shape) and
    /// refreshes the text extractable from it (read on `Ctrl+C` to copy).
    TextSelection {
        anchor: Option<(u32, u32)>,
        end: Option<(u32, u32)>,
        /// Selection shape; ignored when `anchor` / `end` are `None`.
        mode: SelectionMode,
    },
    Quit,
}

/// Canvas-space state of an open `<select>` dropdown, held by the input loop
/// so clicks and keyboard navigation can be resolved without asking the
/// render thread.
#[derive(Debug, Clone)]
pub struct SelectOpenState {
    /// The `<select>` node id.
    pub node_id: NodeId,
    /// Overlay rectangle (canvas coordinates, directly below the field).
    pub rect: crate::layout::Rect,
    /// `(value, label)` pairs of the options.
    pub options: Vec<(String, String)>,
    /// Index of the currently selected option.
    pub selected: usize,
    /// Index highlighted in the overlay (keyboard / hover cursor).
    pub highlight: usize,
}

/// Input-loop state that persists between events.
///
/// Groups the variables the input loop mutates (`scroll_y`, `zoom`) with the
/// mouse-tracking state (press position, hover target), the active
/// `<input>` edit session (focused node, value, caret) and the page text
/// selection (drag selection over normal content), so the loop only passes one
/// state value around.
#[derive(Debug)]
pub struct InputState {
    pub scroll_y: u32,
    pub zoom: f32,
    /// Screen coordinates of the last left-button press; a click is only
    /// recognised when the release lands on the same cell.
    pub pressed: Option<(u16, u16)>,
    /// The `<input>` node currently focused (receives typed characters).
    pub focused_input: Option<NodeId>,
    /// Edit buffer of the focused input; the DOM `value` attribute is the
    /// initial content.
    pub input_value: String,
    /// Caret position in [`InputState::input_value`] (character index).
    pub input_cursor: usize,
    /// Selection anchor (character index). `Some(a)` selects the character
    /// range between `a` and [`InputState::input_cursor`]; `None` means no
    /// selection.
    pub input_sel_anchor: Option<usize>,
    /// Session clipboard used by `Ctrl+C` / `Ctrl+X` / `Ctrl+V`. Terminal-
    /// local only (no OSC 52 / system clipboard integration).
    pub input_clipboard: String,
    /// Whether the pointer is mid-drag inside a focused `<input>` (mouse
    /// selection as opposed to a plain click).
    pub drag_selecting: bool,
    /// Canvas-space anchor of the page text selection (start point; `None`
    /// when no text selection is active).
    pub text_sel_anchor: Option<(u32, u32)>,
    /// Canvas-space current endpoint of the page text selection.
    pub text_sel_end: Option<(u32, u32)>,
    /// Shape of the active page text selection: left-drag is flow (reading
    /// order), right-drag is a block rectangle.
    pub text_sel_mode: SelectionMode,
    /// Text inside the current page selection, refreshed by the render thread
    /// on every `TextSelection` command; read on `Ctrl+C` to copy. Shared
    /// with the render thread through `Arc<Mutex<_>>`.
    pub selection_text: Arc<Mutex<String>>,
    /// The `<select>` dropdown currently open (`None` = closed). While set,
    /// clicks inside the overlay pick an option and clicks outside close it;
    /// `↑` / `↓` / `Enter` / `Esc` navigate it.
    pub open_select: Option<SelectOpenState>,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            scroll_y: 0,
            zoom: 1.0,
            pressed: None,
            focused_input: None,
            input_value: String::new(),
            input_cursor: 0,
            input_sel_anchor: None,
            input_clipboard: String::new(),
            drag_selecting: false,
            text_sel_anchor: None,
            text_sel_end: None,
            text_sel_mode: SelectionMode::Flow,
            selection_text: Arc::new(Mutex::new(String::new())),
            open_select: None,
        }
    }
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

/// Apply one text-editing key to an input field's value and caret.
///
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

/// Byte offset of the `char_idx`-th character (clamped to the end).
fn char_to_byte(s: &str, char_idx: usize) -> usize {
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
fn apply_ctrl_shortcut(state: &mut InputState, key: KeyCode) -> bool {
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
            let byte = char_to_byte(&state.input_value, state.input_cursor);
            state.input_value.insert_str(byte, &text);
            state.input_cursor = state
                .input_cursor
                .saturating_add(text.chars().count());
            state.input_sel_anchor = None;
            true
        }
        _ => false,
    }
}

/// Extend / shrink the selection with `Shift` + navigation, remembering the
/// anchor when the selection starts. Returns `true` when the caret moved.
fn apply_shift_navigation(state: &mut InputState, key: KeyCode) -> bool {
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

/// Focus `node_id` (an `<input>`), seeding the edit buffer from the DOM
/// `value` attribute and repainting the previously focused field (if any).
/// Queues `focus` on the newly focused field and `blur` / `change` on the
/// previously focused one as script messages.
fn focus_input(
    state: &mut InputState,
    node_id: NodeId,
    arena: &NodeArena,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    if state.focused_input == Some(node_id) {
        return;
    }
    if let Some(prev) = state.focused_input.replace(node_id) {
        // Commit the previous field's edits back to the DOM before switching
        // focus, firing `blur` (+ `change` when the value differs).
        msgs.push(ScriptMsg::BlurInput {
            node_id: prev,
            value: state.input_value.clone(),
        });
        state.input_sel_anchor = None;
        cmds.push(RenderCmd::RenderInput {
            node_id: prev,
            value: std::mem::take(&mut state.input_value),
            cursor: std::mem::take(&mut state.input_cursor),
            focused: false,
            selection: None,
        });
    }
    let value = arena
        .get(node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned()
        .unwrap_or_default();
    let cursor = value.chars().count();
    state.input_value = value;
    state.input_cursor = cursor;
    state.input_sel_anchor = None;
    cmds.push(RenderCmd::RenderInput {
        node_id,
        value: state.input_value.clone(),
        cursor,
        focused: true,
        selection: None,
    });
    // Fire `focus` on the newly focused field.
    msgs.push(ScriptMsg::FocusInput { node_id });
}

/// Blur the focused `<input>`: repaint it unfocused, commit the edited value
/// back into the DOM (so a later focus or re-render sees it), and clear the
/// edit session. The commit and the `blur` / `change` events themselves run on
/// the script thread via [`ScriptMsg::BlurInput`].
fn blur_input(
    state: &mut InputState,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    let Some(focused) = state.focused_input.take() else {
        return;
    };
    state.input_sel_anchor = None;
    msgs.push(ScriptMsg::BlurInput {
        node_id: focused,
        value: state.input_value.clone(),
    });
    cmds.push(RenderCmd::RenderInput {
        node_id: focused,
        value: std::mem::take(&mut state.input_value),
        cursor: std::mem::take(&mut state.input_cursor),
        focused: false,
        selection: None,
    });
}

/// Queue a repaint of the focused field with its current edit state.
fn push_input_render(state: &InputState, focused: NodeId, cmds: &mut Vec<RenderCmd>) {
    cmds.push(RenderCmd::RenderInput {
        node_id: focused,
        value: state.input_value.clone(),
        cursor: state.input_cursor,
        focused: true,
        selection: state
            .input_sel_anchor
            .map(|anchor| selection_range(anchor, state.input_cursor)),
    });
}

/// Handle a completed left-button click at canvas coordinates `(x, y)`.
///
/// Queues the `Click` dispatch (the script thread runs it via
/// [`interaction::on_click`], which also syncs the hover state) and then
/// applies the field-focus rule used by the input loop: a click on an
/// `<input>` focuses it (blurring the previously focused field), any other
/// click blurs the focused field. `arena` is the input loop's latest DOM
/// snapshot for the read-only node-type / value lookups; everything that
/// needs the engine is collected into `msgs` for the script thread. Local
/// render commands are appended to `cmds` in dispatch order.
pub fn handle_click(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    x: u32,
    y: u32,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    msgs.push(ScriptMsg::Click { x, y });
    // Clicking an `<input>` focuses it; any other click blurs the previously
    // focused field.
    let clicked = hit_map.hit_test(x, y);
    let clicked_input = clicked.is_some_and(|r| {
        arena
            .get(r.node_id)
            .is_some_and(|n| n.node_type == NodeType::Input)
    });
    if clicked_input {
        if let Some(node_id) = clicked.map(|r| r.node_id) {
            focus_input(state, node_id, arena, msgs, cmds);
        }
    } else if state.focused_input.is_some() {
        blur_input(state, msgs, cmds);
    }
}

/// Map a field-inner column offset (0-based, relative to the first content
/// column after `[`) to a character index, using the same caret-centred
/// window the painter draws. Pointer positions outside the visible window
/// clamp to the nearest reachable character so the caret never leaves the
/// visible slice.
fn char_index_at_x(value: &str, cursor: usize, inner_width: usize, x: usize) -> usize {
    let (start, end) = value_window(value, cursor, inner_width);
    let v: Vec<char> = value.chars().collect();
    let mut col = 0usize;
    for i in start..end {
        let w = char_width(v[i]).max(1);
        if x < col + w {
            return i;
        }
        col += w;
    }
    // Past the visible window's right edge: the window's end is the nearest
    // reachable caret (the caret itself is always inside the window).
    if end < v.len() {
        end
    } else {
        v.len()
    }
}

/// Clear the page text selection, queueing a repaint only when one existed.
fn clear_text_selection(state: &mut InputState, cmds: &mut Vec<RenderCmd>) {
    if state.text_sel_anchor.is_some() || state.text_sel_end.is_some() {
        state.text_sel_anchor = None;
        state.text_sel_end = None;
        cmds.push(RenderCmd::TextSelection {
            anchor: None,
            end: None,
            mode: state.text_sel_mode,
        });
    }
}

/// The overlay option row containing the canvas point `(x, y)`, if any.
fn hit_select_option(open: &SelectOpenState, x: u32, y: u32) -> Option<usize> {
    if y >= open.rect.y as u32
        && y < open.rect.y as u32 + open.rect.height as u32
        && x >= open.rect.x as u32
        && x < open.rect.x as u32 + open.rect.width as u32
    {
        Some((y - open.rect.y as u32) as usize)
    } else {
        None
    }
}

/// Open the `<select>` dropdown for `node_id`: resolve the options and
/// overlay geometry from the DOM, remember them on the input loop, and ask
/// the render thread to paint the overlay.
fn open_select(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    node_id: NodeId,
    cmds: &mut Vec<RenderCmd>,
) {
    let Some(region) = hit_map.iter().find(|r| r.node_id == node_id) else {
        return;
    };
    let options = select_options(arena, node_id);
    let value = arena
        .get(node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned()
        .unwrap_or_default();
    let selected = options
        .iter()
        .position(|(v, _)| *v == value)
        .unwrap_or(0);
    let rect = select_overlay_rect(&options, region.rect);
    state.open_select = Some(SelectOpenState {
        node_id,
        rect,
        options,
        selected,
        highlight: selected,
    });
    cmds.push(RenderCmd::SelectOpen { node_id });
}

/// Close the open `<select>` dropdown. When `pick` is `Some`, the option is
/// selected: its value is committed to the DOM (`set_value_dirty`) and a
/// `change` event fires when the value actually changed; the commit + event
/// run on the script thread via [`ScriptMsg::SelectPick`]. `pick: None` just
/// closes without changing the value.
fn close_select(
    state: &mut InputState,
    arena: &NodeArena,
    pick: Option<usize>,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    let Some(open) = state.open_select.take() else {
        return;
    };
    cmds.push(RenderCmd::SelectClose {
        node_id: open.node_id,
    });
    if let Some(idx) = pick {
        if idx < open.options.len() {
            let value = open.options[idx].0.clone();
            let old = arena
                .get(open.node_id)
                .and_then(|n| n.attributes.get("value"))
                .cloned()
                .unwrap_or_default();
            if value != old {
                msgs.push(ScriptMsg::SelectPick {
                    node_id: open.node_id,
                    value,
                });
            }
        }
    }
}

/// Handle a left-button press at canvas coordinates `(x, y)`.
///
/// Browser-aligned mousedown behaviour: a press on an `<input>` focuses it
/// (blurring any previously focused field) and positions the caret at the
/// press point, entering drag-selection mode; a press anywhere else blurs the
/// focused field and starts a page text selection (the anchor for the drag).
/// Any prior page selection is cleared. `arena` is the input loop's latest DOM
/// snapshot; everything that needs the engine is collected into `msgs` for the
/// script thread. Local render commands are appended to `cmds` in dispatch
/// order.
pub fn handle_mouse_down(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    x: u32,
    y: u32,
    button: MouseButton,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    let clicked = hit_map.hit_test(x, y);

    // An open dropdown takes priority: a press on an option row picks it,
    // a press on the field toggles it closed, any other press closes it and
    // then falls through to the normal dispatch.
    if let Some(open) = state.open_select.clone() {
        if let Some(idx) = hit_select_option(&open, x, y) {
            close_select(state, arena, Some(idx), msgs, cmds);
            return;
        }
        let clicked_self = clicked.is_some_and(|r| r.node_id == open.node_id);
        close_select(state, arena, None, msgs, cmds);
        if clicked_self {
            return;
        }
    }

    let clicked_input = clicked.is_some_and(|r| {
        arena
            .get(r.node_id)
            .is_some_and(|n| n.node_type == NodeType::Input)
    });
    match button {
        MouseButton::Left => {
            if clicked_input {
                if let Some(node_id) = clicked.map(|r| r.node_id) {
                    focus_input(state, node_id, arena, msgs, cmds);
                    state.drag_selecting = true;
                    // Position the caret (and the selection anchor) at the press
                    // point within the field.
                    if let Some(region) = clicked {
                        let inner_x = region.rect.x.saturating_add(1) as usize;
                        let inner_w = region.rect.width.saturating_sub(2) as usize;
                        let idx = char_index_at_x(
                            &state.input_value,
                            state.input_cursor,
                            inner_w,
                            (x as usize).saturating_sub(inner_x),
                        );
                        state.input_sel_anchor = Some(idx);
                        state.input_cursor = idx;
                        push_input_render(state, node_id, cmds);
                    }
                }
            } else {
                // Mousedown outside a field blurs it (browser timing), then
                // starts a flow text selection anchored at the press point.
                if state.focused_input.is_some() {
                    blur_input(state, msgs, cmds);
                }
                clear_text_selection(state, cmds);
                state.text_sel_anchor = Some((x, y));
                state.text_sel_end = Some((x, y));
                state.text_sel_mode = SelectionMode::Flow;
            }
        }
        // Right-drag starts a rectangular (block) selection; the focused field
        // and its caret stay untouched.
        MouseButton::Right => {
            clear_text_selection(state, cmds);
            state.text_sel_anchor = Some((x, y));
            state.text_sel_end = Some((x, y));
            state.text_sel_mode = SelectionMode::Block;
        }
        _ => {}
    }

    // A left press on a closed `<select>` opens its dropdown (blurring any
    // focused field, like a browser mousedown on a control).
    if button == MouseButton::Left
        && let Some(region) = clicked
        && arena
            .get(region.node_id)
            .is_some_and(|n| n.node_type == NodeType::Select)
    {
        if state.focused_input.is_some() {
            blur_input(state, msgs, cmds);
        }
        open_select(state, hit_map, arena, region.node_id, cmds);
    }
}

/// Handle a pointer move at canvas coordinates `(x, y)`.
///
/// While the left button is held this drives the active drag: inside a
/// focused `<input>` the caret (and with it the character selection) follows
/// the pointer; over page content the text selection rectangle tracks the
/// pointer. Without a press it queues a [`ScriptMsg::Move`] for the script
/// thread's hover machinery (pointer enter / leave + `MouseOver` /
/// `MouseOut`). Local render commands are appended to `cmds` in dispatch
/// order.
pub fn handle_mouse_move(
    state: &mut InputState,
    hit_map: &HitMap,
    x: u32,
    y: u32,
    msgs: &mut Vec<ScriptMsg>,
    cmds: &mut Vec<RenderCmd>,
) {
    // An open `<select>` dropdown follows the pointer: hovering an option row
    // moves the highlight (and repaints only the overlay). This runs even
    // while the button is held (browser select: press-drag over the options)
    // and short-circuits when the highlight is unchanged.
    if let Some(idx) = state
        .open_select
        .as_ref()
        .and_then(|open| hit_select_option(open, x, y))
        .filter(|&idx| state.open_select.as_ref().is_some_and(|o| o.highlight != idx))
    {
        if let Some(open) = &mut state.open_select {
            open.highlight = idx;
            cmds.push(RenderCmd::SelectMove {
                node_id: open.node_id,
                highlight: idx,
            });
        }
    }

    if state.pressed.is_some() {
        // Dragging: the pointer is inside the field it started in, so the
        // selection stays within that field's rectangle.
        if state.drag_selecting {
            if let Some(focused) = state.focused_input
                && let Some(region) = hit_map.iter().find(|r| r.node_id == focused)
            {
                let inner_x = region.rect.x.saturating_add(1) as usize;
                let inner_w = region.rect.width.saturating_sub(2) as usize;
                let idx = char_index_at_x(
                    &state.input_value,
                    state.input_cursor,
                    inner_w,
                    (x as usize).saturating_sub(inner_x),
                );
                state.input_cursor = idx;
                push_input_render(state, focused, cmds);
            }
        } else if let Some(anchor) = state.text_sel_anchor {
            state.text_sel_end = Some((x, y));
            cmds.push(RenderCmd::TextSelection {
                anchor: Some(anchor),
                end: Some((x, y)),
                mode: state.text_sel_mode,
            });
        }
        return;
    }
    // No button held: ordinary pointer movement → hover tracking on the
    // script thread.
    msgs.push(ScriptMsg::Move { x, y });
}

/// Wait for the next input event and translate it into render commands.
///
/// The input loop never executes JS: everything that needs the engine is
/// queued into `msgs` (forwarded to the dedicated script thread, so a blocking
/// host function never stalls input); DOM reads use `arena`, the input loop's
/// latest snapshot.
///
/// `timeout` controls how long to block on `event::poll`:
/// - `None` blocks indefinitely (legacy behaviour, suitable for tests or
///   one-shot tools that don't need to react to the render thread dying).
/// - `Some(duration)` returns an empty vec when no event arrives within the
///   duration, giving the caller a chance to check other state (e.g.
///   whether the render thread is still alive).
///
/// `max_scroll` is the largest `scroll_y` the canvas currently allows
/// (i.e. `rendered_rows - viewport_rows`). It is supplied by the render
/// thread after each layout pass. Scroll events that would push past the
/// boundary in either direction are dropped here at the source instead of
/// being forwarded as a no-op `Rerender` — the render thread already had a
/// guard for that, but every dropped command here is one fewer channel
/// send and, more importantly, one fewer frame that risks a visible flash
/// at the scroll bounds.
///
/// `hit_map` and `arena` activate mouse interaction (pass both together,
/// or neither):
/// - left `Down` records the press position and (browser-aligned) focuses an
///   `<input>` / blurs the focused field / starts a drag selection;
/// - `Moved` while the button is held drives the active drag (field-internal
///   character selection or the page text selection); without a press it
///   queues a [`ScriptMsg::Move`] for the script thread's hover machinery
///   (`MouseOver` / `MouseOut`, via
///   [`interaction::on_move`](crate::events::interaction::on_move)). Like a
///   browser without CSS `:hover`, moving the pointer never changes what is
///   painted;
/// - left `Up` on the same cell queues a bubbling `Click` (via
///   [`interaction::on_click`](crate::events::interaction::on_click)); a drag
///   that ends elsewhere keeps the selection.
pub fn trigger_refresh(
    state: &mut InputState,
    rows: u16,
    max_scroll: u32,
    hit_map: Option<&HitMap>,
    arena: Option<&NodeArena>,
    msgs: &mut Vec<ScriptMsg>,
    timeout: Option<Duration>,
) -> Vec<RenderCmd> {
    let mut cmds = Vec::new();
    if let Some(t) = timeout {
        // `poll` returns `Ok(false)` on timeout and `Err` on a system error;
        // both cases should be surfaced as an empty vec so the outer loop can
        // re-check the render thread instead of blocking forever.
        if !event::poll(t).unwrap_or(false) {
            return cmds;
        }
    }
    match event::read() {
        Ok(Event::Key(key)) => {
            if key.kind == KeyEventKind::Release {
                return cmds;
            }

            // An open `<select>` dropdown consumes navigation keys: ↑ / ↓
            // move the highlight, `Enter` picks the highlighted option, `Esc`
            // closes. Any other key closes the dropdown and is swallowed
            // (the dropdown owns the keyboard while open).
            if state.open_select.is_some() {
                let Some(arena) = arena else {
                    return cmds;
                };
                let node_id = state
                    .open_select
                    .as_ref()
                    .map(|o| o.node_id)
                    .unwrap();
                match key.code {
                    KeyCode::Up => {
                        if let Some(open) = &mut state.open_select {
                            let n = open.options.len().max(1);
                            open.highlight = if open.highlight == 0 {
                                n - 1
                            } else {
                                open.highlight - 1
                            };
                            cmds.push(RenderCmd::SelectMove {
                                node_id,
                                highlight: open.highlight,
                            });
                        }
                        return cmds;
                    }
                    KeyCode::Down => {
                        if let Some(open) = &mut state.open_select {
                            let n = open.options.len().max(1);
                            open.highlight = (open.highlight + 1).min(n - 1);
                            cmds.push(RenderCmd::SelectMove {
                                node_id,
                                highlight: open.highlight,
                            });
                        }
                        return cmds;
                    }
                    KeyCode::Enter => {
                        let pick = state.open_select.as_ref().map(|o| o.highlight);
                        close_select(state, arena, pick, msgs, &mut cmds);
                        return cmds;
                    }
                    KeyCode::Esc => {
                        close_select(state, arena, None, msgs, &mut cmds);
                        return cmds;
                    }
                    _ => {
                        close_select(state, arena, None, msgs, &mut cmds);
                        return cmds;
                    }
                }
            }

            // A focused `<input>` consumes the keys it owns: text editing and
            // caret movement. `Esc` blurs it and `Enter` confirms (blur +
            // commit) — both write the value back to the DOM. Keys the field
            // does *not* own (page navigation such as ↓ / ↑ / PageDown /
            // PageUp) fall through to the page-level handler below, matching
            // a browser where an input only intercepts the keys it edits.
            if state.focused_input.is_some() {
                if key.code == KeyCode::Esc || key.code == KeyCode::Enter {
                    blur_input(state, msgs, &mut cmds);
                    return cmds;
                }
                let Some(focused) = state.focused_input else {
                    return cmds;
                };

                // Ctrl shortcuts: select-all / copy / cut / paste. Other Ctrl
                // combos are ignored (not inserted as characters — browser
                // behaviour).
                if key.modifiers.contains(event::KeyModifiers::CONTROL) {
                    if apply_ctrl_shortcut(state, key.code) {
                        push_input_render(state, focused, &mut cmds);
                        if arena.is_some() {
                            msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
                        }
                    }
                    return cmds;
                }

                // Shift + navigation extends / shrinks the selection.
                if key.modifiers.contains(event::KeyModifiers::SHIFT) {
                    if apply_shift_navigation(state, key.code) {
                        push_input_render(state, focused, &mut cmds);
                    }
                    return cmds;
                }

                // Field-owned keys stay in the field even when they are
                // boundary no-ops (Home at the start must not scroll the
                // page). Everything else belongs to the page-level handler.
                if is_input_field_key(key.code) {
                    // Plain editing keys clear the selection.
                    if apply_input_key(&mut state.input_value, &mut state.input_cursor, key.code) {
                        state.input_sel_anchor = None;
                        push_input_render(state, focused, &mut cmds);
                        if arena.is_some() {
                            msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
                        }
                    }
                    return cmds;
                }
                // Fall through to the page-level key handling below.
            }

            if key.modifiers.contains(event::KeyModifiers::CONTROL) {
                match key.code {
                    KeyCode::Char('=') | KeyCode::Char('+') => {
                        state.zoom = (state.zoom + ZOOM_STEP).min(ZOOM_MAX);
                        return vec![RenderCmd::Redraw {
                            scroll_y: state.scroll_y,
                            zoom: state.zoom,
                        }];
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        state.zoom = (state.zoom - ZOOM_STEP).max(ZOOM_MIN);
                        return vec![RenderCmd::Redraw {
                            scroll_y: state.scroll_y,
                            zoom: state.zoom,
                        }];
                    }
                    KeyCode::Char('0') => {
                        state.zoom = 1.0;
                        return vec![RenderCmd::Redraw {
                            scroll_y: state.scroll_y,
                            zoom: state.zoom,
                        }];
                    }
                    // Copy a page text selection (inside an `<input>` the
                    // Ctrl shortcuts are handled earlier). `Ctrl+X` on read-
                    // only page text degrades to a copy, like a browser.
                    KeyCode::Char('c') | KeyCode::Char('x') => {
                        if state.focused_input.is_none() && state.text_sel_anchor.is_some() {
                            if let Ok(guard) = state.selection_text.lock() {
                                state.input_clipboard = guard.clone();
                            }
                        }
                    }
                    _ => {}
                }
            } else if state.text_sel_anchor.is_some() {
                // A non-Ctrl key abandons the page text selection (the
                // selection is a pointer-driven, transient visual state).
                clear_text_selection(state, &mut cmds);
            }

            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => cmds.push(RenderCmd::Quit),

                KeyCode::Up | KeyCode::Char('k') => {
                    // Already at the top boundary — drop the event instead
                    // of forwarding a no-op scroll to the render thread.
                    if state.scroll_y == 0 {
                        return cmds;
                    }
                    state.scroll_y -= 1;
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    // Already at the bottom boundary — drop the event.
                    if state.scroll_y >= max_scroll {
                        return cmds;
                    }
                    state.scroll_y += 1;
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                KeyCode::PageUp => {
                    if state.scroll_y == 0 {
                        return cmds;
                    }
                    let step = (rows as u32).saturating_sub(1).max(1);
                    state.scroll_y = state.scroll_y.saturating_sub(step);
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                KeyCode::PageDown => {
                    if state.scroll_y >= max_scroll {
                        return cmds;
                    }
                    let step = (rows as u32).saturating_sub(1).max(1);
                    state.scroll_y = state.scroll_y.saturating_add(step);
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                KeyCode::Home => {
                    // No-op move — drop to avoid a wasted rerender.
                    if state.scroll_y == 0 {
                        return cmds;
                    }
                    state.scroll_y = 0;
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                KeyCode::End => {
                    // No-op move (already at the bottom) — drop.
                    if state.scroll_y >= max_scroll {
                        return cmds;
                    }
                    // Use the actual upper bound rather than u32::MAX so the
                    // render thread receives a value it doesn't have to clamp.
                    state.scroll_y = max_scroll;
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                _ => {}
            }
            cmds
        }

        Ok(Event::Mouse(mouse)) => {
            let ctrl = mouse.modifiers.contains(event::KeyModifiers::CONTROL);
            match mouse.kind {
                MouseEventKind::ScrollUp => {
                    // Scrolling dismisses an open dropdown (focus loss).
                    if state.open_select.is_some()
                        && let Some(arena) = arena
                    {
                        close_select(state, arena, None, msgs, &mut cmds);
                    }
                    if ctrl {
                        state.zoom = (state.zoom - ZOOM_STEP).max(ZOOM_MIN);
                        return vec![RenderCmd::Redraw {
                            scroll_y: state.scroll_y,
                            zoom: state.zoom,
                        }];
                    }
                    if state.scroll_y == 0 {
                        return cmds;
                    }
                    state.scroll_y = state.scroll_y.saturating_sub(SCROLL_LINES_PER_TICK);
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                MouseEventKind::ScrollDown => {
                    // Scrolling dismisses an open dropdown (focus loss).
                    if state.open_select.is_some()
                        && let Some(arena) = arena
                    {
                        close_select(state, arena, None, msgs, &mut cmds);
                    }
                    if ctrl {
                        state.zoom = (state.zoom + ZOOM_STEP).min(ZOOM_MAX);
                        return vec![RenderCmd::Redraw {
                            scroll_y: state.scroll_y,
                            zoom: state.zoom,
                        }];
                    }
                    if state.scroll_y >= max_scroll {
                        return cmds;
                    }
                    state.scroll_y = state.scroll_y.saturating_add(SCROLL_LINES_PER_TICK);
                    cmds.push(RenderCmd::Rerender {
                        scroll_y: state.scroll_y,
                    });
                }
                MouseEventKind::Down(button) => {
                    if let (Some(hit_map), Some(arena)) = (hit_map, arena) {
                        state.pressed = Some((mouse.column, mouse.row));
                        let (x, y) = canvas_pos(state, mouse.column, mouse.row);
                        handle_mouse_down(state, hit_map, arena, x, y, button, msgs, &mut cmds);
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    if let (Some((px, py)), Some(hit_map), Some(arena)) =
                        (state.pressed, hit_map, arena)
                    {
                        state.pressed = None;
                        state.drag_selecting = false;
                        let (x, y) = canvas_pos(state, mouse.column, mouse.row);
                        // Press-drag-release over a dropdown option picks it
                        // (the browser's native `<select>` interaction), even
                        // when the release lands off the press cell.
                        if let Some(open) = state.open_select.clone() {
                            if let Some(idx) = hit_select_option(&open, x, y) {
                                close_select(state, arena, Some(idx), msgs, &mut cmds);
                                return cmds;
                            }
                        }
                        if px == mouse.column && py == mouse.row {
                            handle_click(state, hit_map, arena, x, y, msgs, &mut cmds);
                        }
                        // A drag ended on a different cell: the selection
                        // (field-internal or page text) is kept as-is.
                    } else {
                        state.pressed = None;
                    }
                }
                MouseEventKind::Up(MouseButton::Right) | MouseEventKind::Up(MouseButton::Middle) => {
                    // Block selection (right button) ends here; the selection
                    // itself is kept. No click is dispatched for these buttons.
                    state.pressed = None;
                    state.drag_selecting = false;
                }
                // crossterm reports button-held motion as `Drag` (SGR code
                // 32..34) and button-free motion as `Moved` (SGR 64) — both
                // must reach the drag / hover handler.
                MouseEventKind::Drag(_) | MouseEventKind::Moved => {
                    if let (Some(hit_map), Some(_)) = (hit_map, arena) {
                        let (x, y) = canvas_pos(state, mouse.column, mouse.row);
                        handle_mouse_move(state, hit_map, x, y, msgs, &mut cmds);
                    }
                }
                _ => {}
            }
            cmds
        }

        Ok(Event::Resize(_, _new_rows)) => vec![RenderCmd::Redraw {
            scroll_y: state.scroll_y,
            zoom: state.zoom,
        }],

        _ => cmds,
    }
}

/// Convert a screen position to canvas coordinates by adding the scroll
/// offset. Zoom is not yet wired into layout, so no zoom scaling is applied.
fn canvas_pos(state: &InputState, column: u16, row: u16) -> (u32, u32) {
    (column as u32, row as u32 + state.scroll_y)
}
