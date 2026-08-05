use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind};

use crate::{
    events::interaction,
    script::ScriptEngine,
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::char_width,
};
use crate::events::{ElementRef, EventType, HitMap, HitRegion};
use crate::layout::paint::value_window;
use crate::layout::Rect;

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
    /// keeping the current scroll position / zoom. The render thread swaps
    /// the document and diff-renders, so handlers can build dynamic content
    /// (e.g. a counter that re-renders on every click).
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
    /// Hover tracking state for mouse enter / leave detection.
    pub hover: interaction::HoverState,
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
}

impl InputState {
    pub fn new() -> Self {
        Self {
            scroll_y: 0,
            zoom: 1.0,
            pressed: None,
            hover: interaction::HoverState::new(),
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

/// Build a minimal hit region for `node_id` (no geometry) so the input loop
/// can fire focus / blur / input / change events along the DOM ancestor chain,
/// exactly like a pointer-driven region from the hit map.
fn region_for(arena: &NodeArena, node_id: NodeId) -> HitRegion {
    let mut path = Vec::new();
    let mut current = Some(node_id);
    while let Some(id) = current {
        let node = &arena[id];
        path.push(ElementRef {
            id: node.get_id().cloned(),
            node_id: id,
        });
        current = node.parent;
    }
    HitRegion {
        node_id,
        rect: Rect::new(0, 0, 0, 0),
        path,
    }
}

/// Fire `blur` (+ `change` when the edited value differs from the committed
/// DOM value) for `node_id`, committing `new_value` into the DOM first.
/// Returns any render commands pushed by handlers.
fn fire_blur_and_change(
    engine: &mut ScriptEngine,
    node_id: NodeId,
    new_value: &str,
) -> Vec<RenderCmd> {
    let arena = engine.arena();
    let old = arena
        .get(node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned()
        .unwrap_or_default();
    engine.set_value(node_id, new_value);
    let region = region_for(&arena, node_id);
    let mut cmds = engine.fire(&region, EventType::Blur, 0, 0);
    if old != new_value {
        cmds.extend(engine.fire(&region, EventType::Change, 0, 0));
    }
    cmds
}

/// Focus `node_id` (an `<input>`), seeding the edit buffer from the DOM
/// `value` attribute and repainting the previously focused field (if any).
/// Fires `focus` on the newly focused field and `blur` / `change` on the
/// previously focused one.
fn focus_input(
    state: &mut InputState,
    node_id: NodeId,
    engine: &mut ScriptEngine,
    cmds: &mut Vec<RenderCmd>,
) {
    if state.focused_input == Some(node_id) {
        return;
    }
    if let Some(prev) = state.focused_input.replace(node_id) {
        // Commit the previous field's edits back to the DOM before switching
        // focus, firing `blur` (+ `change` when the value differs).
        cmds.extend(fire_blur_and_change(engine, prev, &state.input_value));
        state.input_sel_anchor = None;
        cmds.push(RenderCmd::RenderInput {
            node_id: prev,
            value: std::mem::take(&mut state.input_value),
            cursor: std::mem::take(&mut state.input_cursor),
            focused: false,
            selection: None,
        });
    }
    let value = engine
        .arena()
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
    let region = region_for(&engine.arena(), node_id);
    cmds.extend(engine.fire(&region, EventType::Focus, 0, 0));
}

/// Blur the focused `<input>`: repaint it unfocused, commit the edited value
/// back into the DOM (so a later focus or re-render sees it), and clear the
/// edit session. `engine` is `None` in the no-event mode, where there is no
/// live DOM to write back to.
fn blur_input(
    state: &mut InputState,
    engine: Option<&mut ScriptEngine>,
    cmds: &mut Vec<RenderCmd>,
) {
    let Some(focused) = state.focused_input.take() else {
        return;
    };
    state.input_sel_anchor = None;
    if let Some(engine) = engine {
        cmds.extend(fire_blur_and_change(engine, focused, &state.input_value));
    }
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
/// Dispatches the `Click` event (via [`interaction::on_click`], which also
/// syncs the hover state) and then applies the field-focus rule used by the
/// input loop: a click on an `<input>` focuses it (blurring the previously
/// focused field), any other click blurs the focused field. Render commands
/// are appended to `cmds` in dispatch order.
pub fn handle_click(
    state: &mut InputState,
    hit_map: &HitMap,
    engine: &mut ScriptEngine,
    x: u32,
    y: u32,
    cmds: &mut Vec<RenderCmd>,
) {
    cmds.extend(interaction::on_click(&mut state.hover, hit_map, engine, x, y));
    // Clicking an `<input>` focuses it; any other click blurs the previously
    // focused field.
    let clicked = hit_map.hit_test(x, y);
    let clicked_input = clicked.is_some_and(|r| {
        engine
            .arena()
            .get(r.node_id)
            .is_some_and(|n| n.node_type == NodeType::Input)
    });
    if clicked_input {
        if let Some(node_id) = clicked.map(|r| r.node_id) {
            focus_input(state, node_id, engine, cmds);
        }
    } else if state.focused_input.is_some() {
        blur_input(state, Some(engine), cmds);
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

/// Handle a left-button press at canvas coordinates `(x, y)`.
///
/// Browser-aligned mousedown behaviour: a press on an `<input>` focuses it
/// (blurring any previously focused field) and positions the caret at the
/// press point, entering drag-selection mode; a press anywhere else blurs the
/// focused field and starts a page text selection (the anchor for the drag).
/// Any prior page selection is cleared. Render commands are appended to `cmds`
/// in dispatch order.
pub fn handle_mouse_down(
    state: &mut InputState,
    hit_map: &HitMap,
    engine: &mut ScriptEngine,
    x: u32,
    y: u32,
    button: MouseButton,
    cmds: &mut Vec<RenderCmd>,
) {
    let clicked = hit_map.hit_test(x, y);
    let clicked_input = clicked.is_some_and(|r| {
        engine
            .arena()
            .get(r.node_id)
            .is_some_and(|n| n.node_type == NodeType::Input)
    });
    match button {
        MouseButton::Left => {
            if clicked_input {
                if let Some(node_id) = clicked.map(|r| r.node_id) {
                    focus_input(state, node_id, engine, cmds);
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
                    blur_input(state, Some(engine), cmds);
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
}

/// Handle a pointer move at canvas coordinates `(x, y)`.
///
/// While the left button is held this drives the active drag: inside a
/// focused `<input>` the caret (and with it the character selection) follows
/// the pointer; over page content the text selection rectangle tracks the
/// pointer. Without a press it is a plain pointer move, dispatched to the
/// hover machinery as usual.
pub fn handle_mouse_move(
    state: &mut InputState,
    hit_map: &HitMap,
    engine: &mut ScriptEngine,
    x: u32,
    y: u32,
    cmds: &mut Vec<RenderCmd>,
) {
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
    // No button held: ordinary pointer movement → hover tracking.
    cmds.extend(interaction::on_move(&mut state.hover, hit_map, engine, x, y));
}

/// Wait for the next input event and translate it into render commands.
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
/// `hit_map` and `engine` activate mouse interaction (pass both together,
/// or neither):
/// - left `Down` records the press position and (browser-aligned) focuses an
///   `<input>` / blurs the focused field / starts a drag selection;
/// - `Moved` while the button is held drives the active drag (field-internal
///   character selection or the page text selection); without a press it
///   detects pointer enter / leave and dispatches `MouseOver` / `MouseOut`
///   via [`interaction::on_move`](crate::events::interaction::on_move). Like
///   a browser without CSS `:hover`, moving the pointer never changes what is
///   painted;
/// - left `Up` on the same cell is dispatched as a bubbling `Click` via
///   [`interaction::on_click`](crate::events::interaction::on_click); a drag
///   that ends elsewhere keeps the selection.
pub fn trigger_refresh(
    state: &mut InputState,
    rows: u16,
    max_scroll: u32,
    hit_map: Option<&HitMap>,
    engine: Option<&mut ScriptEngine>,
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

            // A focused `<input>` consumes text-editing keys; `Esc` blurs it
            // and `Enter` confirms (blur + commit) — both write the value back
            // to the DOM. Either returns control to scrolling / quitting.
            if state.focused_input.is_some() {
                if key.code == KeyCode::Esc || key.code == KeyCode::Enter {
                    blur_input(state, engine, &mut cmds);
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
                        if let Some(engine) = engine {
                            let region = region_for(&engine.arena(), focused);
                            cmds.extend(engine.fire(&region, EventType::Input, 0, 0));
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

                // Plain editing keys clear the selection.
                if apply_input_key(&mut state.input_value, &mut state.input_cursor, key.code) {
                    state.input_sel_anchor = None;
                    push_input_render(state, focused, &mut cmds);
                    if let Some(engine) = engine {
                        let region = region_for(&engine.arena(), focused);
                        cmds.extend(engine.fire(&region, EventType::Input, 0, 0));
                    }
                }
                return cmds;
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
                    if let (Some(hit_map), Some(engine)) = (hit_map, engine) {
                        state.pressed = Some((mouse.column, mouse.row));
                        let (x, y) = canvas_pos(state, mouse.column, mouse.row);
                        handle_mouse_down(state, hit_map, engine, x, y, button, &mut cmds);
                    }
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    if let (Some((px, py)), Some(hit_map), Some(engine)) =
                        (state.pressed, hit_map, engine)
                    {
                        state.pressed = None;
                        state.drag_selecting = false;
                        if px == mouse.column && py == mouse.row {
                            let (x, y) = canvas_pos(state, mouse.column, mouse.row);
                            handle_click(state, hit_map, engine, x, y, &mut cmds);
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
                    if let (Some(hit_map), Some(engine)) = (hit_map, engine) {
                        let (x, y) = canvas_pos(state, mouse.column, mouse.row);
                        handle_mouse_move(state, hit_map, engine, x, y, &mut cmds);
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
