use std::sync::{Arc, Mutex};
use std::time::Duration;

use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MediaKeyCode, ModifierKeyCode,
    MouseButton, MouseEventKind,
};

use crate::{
    events::KeyEventInfo,
    script::ScriptMsg,
    tdom::node::{NodeArena, NodeId, NodeType},
    tui::graph_api::char_width,
};
use crate::events::HitMap;
use crate::layout::Rect;
use crate::layout::layoutbox::geometry::{
    SelectRow, overlay_options, select_item_indices, select_overlay_rect, select_rows,
};
use crate::layout::paint::{
    textarea_caret_position, textarea_char_at, textarea_scroll_row, textarea_visual_lines,
    value_window,
};

const SCROLL_LINES_PER_TICK: u32 = 3;
const ZOOM_STEP: f32 = 0.1;
const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 5.0;

/// Convert a crossterm `KeyEvent` into the terminal-agnostic payload the
/// script layer dispatches as `keydown` / `keyup`, mirroring the browser
/// `KeyboardEvent` fields (`key` / `code` / `keyCode` + modifier flags). Every
/// `KeyCode` variant maps to a stable string so pages can branch on the same
/// values they would in a real browser.
pub fn key_event_info(ev: &KeyEvent) -> KeyEventInfo {
    let (key, code, key_code): (String, String, u32) = match ev.code {
        KeyCode::Backspace => ("Backspace".into(), "Backspace".into(), 8),
        KeyCode::Enter => ("Enter".into(), "Enter".into(), 13),
        KeyCode::Left => ("ArrowLeft".into(), "ArrowLeft".into(), 37),
        KeyCode::Right => ("ArrowRight".into(), "ArrowRight".into(), 39),
        KeyCode::Up => ("ArrowUp".into(), "ArrowUp".into(), 38),
        KeyCode::Down => ("ArrowDown".into(), "ArrowDown".into(), 40),
        KeyCode::Home => ("Home".into(), "Home".into(), 36),
        KeyCode::End => ("End".into(), "End".into(), 35),
        KeyCode::PageUp => ("PageUp".into(), "PageUp".into(), 33),
        KeyCode::PageDown => ("PageDown".into(), "PageDown".into(), 34),
        KeyCode::Tab | KeyCode::BackTab => ("Tab".into(), "Tab".into(), 9),
        KeyCode::Delete => ("Delete".into(), "Delete".into(), 46),
        KeyCode::Insert => ("Insert".into(), "Insert".into(), 45),
        KeyCode::F(n) => (format!("F{n}"), format!("F{n}"), 111 + n as u32),
        KeyCode::Char(c) => (c.to_string(), char_code(c), c as u32),
        KeyCode::Null => ("Unidentified".into(), "Unidentified".into(), 0),
        KeyCode::Esc => ("Escape".into(), "Escape".into(), 27),
        KeyCode::CapsLock => ("CapsLock".into(), "CapsLock".into(), 20),
        KeyCode::ScrollLock => ("ScrollLock".into(), "ScrollLock".into(), 145),
        KeyCode::NumLock => ("NumLock".into(), "NumLock".into(), 144),
        KeyCode::PrintScreen => ("PrintScreen".into(), "PrintScreen".into(), 44),
        KeyCode::Pause => ("Pause".into(), "Pause".into(), 19),
        KeyCode::Menu => ("ContextMenu".into(), "ContextMenu".into(), 93),
        KeyCode::KeypadBegin => ("Unidentified".into(), "Unidentified".into(), 0),
        KeyCode::Media(m) => media_key(m),
        KeyCode::Modifier(m) => modifier_key(m),
    };
    KeyEventInfo {
        key,
        code,
        key_code,
        shift: ev.modifiers.contains(event::KeyModifiers::SHIFT),
        ctrl: ev.modifiers.contains(event::KeyModifiers::CONTROL),
        alt: ev.modifiers.contains(event::KeyModifiers::ALT),
        meta: ev.modifiers.contains(event::KeyModifiers::SUPER)
            || ev.modifiers.contains(event::KeyModifiers::HYPER)
            || ev.modifiers.contains(event::KeyModifiers::META),
        repeat: ev.kind == KeyEventKind::Repeat,
    }
}

/// `event.code` for a character key: `KeyA`…`KeyZ`, `Digit0`…`Digit9`, `Space`
/// and `Unidentified` for everything else (matching the DOM physical-key
/// vocabulary).
fn char_code(c: char) -> String {
    if c.is_ascii_alphabetic() {
        format!("Key{}", c.to_ascii_uppercase())
    } else if c.is_ascii_digit() {
        format!("Digit{c}")
    } else if c == ' ' {
        "Space".to_string()
    } else {
        "Unidentified".to_string()
    }
}

/// `(key, code, keyCode)` for a crossterm media key.
fn media_key(m: MediaKeyCode) -> (String, String, u32) {
    let (k, c, code) = match m {
        MediaKeyCode::Play | MediaKeyCode::PlayPause => ("MediaPlayPause", "MediaPlayPause", 179),
        MediaKeyCode::Pause => ("Pause", "MediaPause", 179),
        MediaKeyCode::Reverse => ("MediaReverse", "MediaReverse", 227),
        MediaKeyCode::Stop => ("MediaStop", "MediaStop", 178),
        MediaKeyCode::FastForward => ("MediaFastForward", "MediaFastForward", 228),
        MediaKeyCode::Rewind => ("MediaRewind", "MediaRewind", 227),
        MediaKeyCode::TrackNext => ("MediaTrackNext", "MediaTrackNext", 176),
        MediaKeyCode::TrackPrevious => ("MediaTrackPrevious", "MediaTrackPrevious", 177),
        MediaKeyCode::Record => ("MediaRecord", "MediaRecord", 0),
        MediaKeyCode::LowerVolume => ("AudioVolumeDown", "AudioVolumeDown", 174),
        MediaKeyCode::RaiseVolume => ("AudioVolumeUp", "AudioVolumeUp", 175),
        MediaKeyCode::MuteVolume => ("AudioVolumeMute", "AudioVolumeMute", 173),
    };
    (k.into(), c.into(), code)
}

/// `(key, code, keyCode)` for a crossterm modifier-key press (only readable
/// with the keyboard-enhancement flags enabled on the terminal).
fn modifier_key(m: ModifierKeyCode) -> (String, String, u32) {
    let (k, c, code) = match m {
        ModifierKeyCode::LeftShift => ("Shift", "ShiftLeft", 16),
        ModifierKeyCode::RightShift => ("Shift", "ShiftRight", 16),
        ModifierKeyCode::LeftControl => ("Control", "ControlLeft", 17),
        ModifierKeyCode::RightControl => ("Control", "ControlRight", 17),
        ModifierKeyCode::LeftAlt => ("Alt", "AltLeft", 18),
        ModifierKeyCode::RightAlt => ("Alt", "AltRight", 18),
        ModifierKeyCode::LeftSuper => ("Meta", "MetaLeft", 91),
        ModifierKeyCode::RightSuper => ("Meta", "MetaRight", 91),
        ModifierKeyCode::LeftHyper => ("Hyper", "HyperLeft", 91),
        ModifierKeyCode::RightHyper => ("Hyper", "HyperRight", 91),
        ModifierKeyCode::LeftMeta => ("Meta", "MetaLeft", 91),
        ModifierKeyCode::RightMeta => ("Meta", "MetaRight", 91),
        ModifierKeyCode::IsoLevel3Shift => ("AltGraph", "AltGraph", 18),
        ModifierKeyCode::IsoLevel5Shift => ("Unidentified", "Unidentified", 0),
    };
    (k.into(), c.into(), code)
}

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
    /// Open the dropdown overlay for `node_id`: the render thread resolves the
    /// options (a `<select>`'s children, or an `<input list>`'s `<datalist>`
    /// filtered by `filter`), paints the overlay below the field and
    /// recolours the field's frame. `filter` is `None` for a plain `<select>`.
    SelectOpen {
        node_id: NodeId,
        filter: Option<String>,
    },
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

/// What an open dropdown overlay is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    /// A `<select>` — picking an option commits its value and fires `change`.
    Select,
    /// An `<input list="…">` autocomplete — picking fills the field.
    Datalist,
}

/// Canvas-space state of an open dropdown overlay, held by the input loop so
/// clicks and keyboard navigation can be resolved without asking the render
/// thread.
#[derive(Debug, Clone)]
pub struct SelectOpenState {
    /// The owning `<select>` / `<input>` node id.
    pub node_id: NodeId,
    /// What the overlay is attached to (select vs input autocomplete).
    pub kind: OverlayKind,
    /// Overlay rectangle (canvas coordinates, directly below the field).
    pub rect: crate::layout::Rect,
    /// The flat option list (`value`, `label`, optional group).
    pub options: Vec<crate::layout::layoutbox::geometry::SelectOption>,
    /// The dropdown rows (group headers interleaved with options).
    pub rows: Vec<crate::layout::layoutbox::geometry::SelectRow>,
    /// Index of the currently selected option.
    pub selected: usize,
    /// Index highlighted in the overlay (keyboard / hover cursor).
    pub highlight: usize,
}

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
    /// The `<input>` / `<textarea>` node currently focused (receives typed
    /// characters).
    pub focused_input: Option<NodeId>,
    /// Whether the focused node is a `<textarea>` (multi-line editing:
    /// Enter inserts a newline, ↑/↓ move the caret, click maps to rows).
    pub focused_is_textarea: bool,
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
    /// local only: copy stores a selection, paste inserts it (single-line
    /// fields collapse newlines). System-clipboard paste arrives separately
    /// through bracketed paste (`Event::Paste`).
    pub input_clipboard: String,
    /// Whether the pointer is mid-drag inside a focused `<input>` (mouse
    /// selection as opposed to a plain click).
    pub drag_selecting: bool,
    /// Screen coordinates of the last pointer event (any button state). Lets
    /// the run loop re-validate hover against a freshly rebuilt hit map after
    /// a viewport change by synthesising a `Move` at the known position.
    pub last_mouse: Option<(u16, u16)>,
    /// Set when the viewport changed (resize poll or `Event::Resize`) so the
    /// run loop re-validates the pointer hover once the rebuilt hit map is in
    /// place (consumed by [`InputState::take_resize_revalidation`]).
    pub resize_pending: bool,
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
    /// Monotonic id source for gated `keydown` dispatches; each press echoes
    /// the id back in the script thread's response so the input loop can match
    /// the gate verdict (`preventDefault`).
    pub next_key_id: u32,
    /// A page-level built-in action (scroll / zoom / quit) waiting for its
    /// `keydown` gate verdict (`None` = no key pending). Set by
    /// [`trigger_refresh`], consumed by the run loop after the wait.
    pub key_gate: Option<KeyGate>,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            scroll_y: 0,
            zoom: 1.0,
            pressed: None,
            focused_input: None,
            focused_is_textarea: false,
            input_value: String::new(),
            input_cursor: 0,
            input_sel_anchor: None,
            input_clipboard: String::new(),
            drag_selecting: false,
            last_mouse: None,
            resize_pending: false,
            text_sel_anchor: None,
            text_sel_end: None,
            text_sel_mode: SelectionMode::Flow,
            selection_text: Arc::new(Mutex::new(String::new())),
            open_select: None,
            next_key_id: 0,
            key_gate: None,
        }
    }

    /// A viewport change (terminal resize) invalidates every canvas coordinate
    /// the input loop caches: a press whose release the console dropped would
    /// otherwise stick and turn all later pointer motion into a drag (hover /
    /// click events go permanently silent), and a stale text selection / open
    /// dropdown overlay would highlight or paint stale cells. Resets that
    /// state and emits the render commands that clear the render-side mirrors.
    pub fn invalidate_viewport(&mut self, cmds: &mut Vec<RenderCmd>) {
        self.pressed = None;
        self.drag_selecting = false;
        clear_text_selection(self, cmds);
        if let Some(open) = self.open_select.take() {
            cmds.push(RenderCmd::SelectClose {
                node_id: open.node_id,
            });
        }
    }

    /// Drop a press whose release was never received: a button-free `Moved`
    /// event proves no button is held, so any recorded press is stale. Without
    /// this, a lost release (e.g. the console dropping the `Up` during a
    /// resize) would make every later move look like a drag and hover / click
    /// events would stay silent forever. Returns whether a press was dropped.
    pub fn drop_stale_press(&mut self) -> bool {
        if self.pressed.take().is_some() {
            self.drag_selecting = false;
            true
        } else {
            false
        }
    }

    /// Remember that the viewport changed; the run loop re-validates hover
    /// after the rebuilt hit map is published (see
    /// [`InputState::take_resize_revalidation`]).
    pub fn request_resize_revalidation(&mut self) {
        self.resize_pending = true;
    }

    /// Consume the pending viewport-change flag; `true` when the run loop
    /// should synthesise a `Move` at [`InputState::last_mouse`] against the
    /// fresh hit map.
    pub fn take_resize_revalidation(&mut self) -> bool {
        std::mem::take(&mut self.resize_pending)
    }

    /// Mirror the render thread's scroll clamp: after a terminal grow / a
    /// shorter document reload the content ceiling (`max_scroll`) can drop
    /// below the current [`InputState::scroll_y`]. Keeping the two in sync
    /// prevents the "wheel dead zone" where every scroll `Rerender` is clamped
    /// to the same value in the render thread and dropped as a no-op, making
    /// the mouse wheel and arrow keys appear dead after a resize.
    pub fn clamp_scroll_to(&mut self, max_scroll: u32) {
        if self.scroll_y > max_scroll {
            self.scroll_y = max_scroll;
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
pub fn apply_textarea_key(
    value: &mut String,
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
    value: &mut String,
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
fn textarea_content_width(arena: Option<&NodeArena>, node_id: NodeId) -> usize {
    arena
        .and_then(|a| a.get(node_id))
        .and_then(|n| n.attributes.get("cols"))
        .and_then(|v| v.parse().ok())
        .unwrap_or(20)
        .max(1)
}

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

/// `true` for an `<input type=checkbox|radio>` node — a click toggles the
/// control instead of focusing a text field.
fn is_toggle_input(arena: &NodeArena, node_id: NodeId) -> bool {
    arena.get(node_id).is_some_and(|n| {
        n.get_type()
            .is_some_and(|t| matches!(t.to_ascii_lowercase().as_str(), "checkbox" | "radio"))
    })
}

/// `true` for a `<textarea>` node — the multi-line field whose caret
/// placement uses visual row/col, not the single-line column mapping.
fn is_textarea_node(arena: &NodeArena, node_id: NodeId) -> bool {
    arena
        .get(node_id)
        .is_some_and(|n| n.node_type == NodeType::Textarea)
}

/// Map a pointer cell inside a `<textarea>` rect to a character index, using
/// the same wrapped-line + caret-derived scroll the painter draws. The rect
/// includes the top/bottom border rows; content starts one row in.
fn textarea_index_at(state: &InputState, rect: Rect, x: u32, y: u32) -> usize {
    let content_width = rect.width.saturating_sub(2).max(1);
    let rows = rect.height.saturating_sub(2).max(1);
    let scroll = textarea_scroll_row(
        &state.input_value,
        content_width,
        rows,
        state.input_cursor,
    );
    let local_row = (y as usize)
        .saturating_sub(rect.y.saturating_add(1))
        .saturating_add(scroll);
    let local_col = (x as usize).saturating_sub(rect.x.saturating_add(1));
    textarea_char_at(&state.input_value, content_width, local_row, local_col)
}

/// `true` for a focusable text field (`<input>` / `<textarea>`).
fn is_text_field(arena: &NodeArena, node_id: NodeId) -> bool {
    arena.get(node_id).is_some_and(|n| {
        matches!(n.node_type, NodeType::Input | NodeType::Textarea)
    })
}

/// All `<input>` / `<textarea>` node ids in document order (depth-first), for
/// `Tab` / `Shift`+`Tab` focus cycling. Reads the input loop's DOM snapshot; a
/// removed or not-yet-rendered field is simply not included. `hidden` inputs
/// and checkbox/radio toggles (click-only in this engine) are skipped.
pub fn collect_input_ids(arena: &NodeArena) -> Vec<NodeId> {
    let mut out = Vec::new();
    let Some(root) = arena.iter().find(|(_, n)| n.parent.is_none()).map(|(id, _)| id) else {
        return out;
    };
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        let node = &arena[id];
        match node.node_type {
            NodeType::Textarea => out.push(id),
            NodeType::Input => {
                let t = node
                    .get_type()
                    .map(|t| t.to_ascii_lowercase())
                    .unwrap_or_default();
                if t != "hidden" && t != "checkbox" && t != "radio" {
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
    state.focused_is_textarea = arena
        .get(node_id)
        .is_some_and(|n| n.node_type == NodeType::Textarea);
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
    // `<input>` fields read the DOM `value` attribute; a `<textarea>`'s
    // initial value is its text content.
    let value = match arena.get(node_id) {
        Some(n) if n.node_type == NodeType::Textarea => n.text_content(arena),
        Some(n) => n.attributes.get("value").cloned().unwrap_or_default(),
        None => String::new(),
    };
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
    state.focused_is_textarea = false;
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
    let clicked = hit_map.hit_test(x, y);
    // A disabled form control swallows the click entirely (no Click
    // dispatch, no focus change) — matching browser behaviour.
    if clicked.is_some_and(|r| is_disabled_control(arena, r.node_id)) {
        return;
    }
    msgs.push(ScriptMsg::Click { x, y });
    // Clicking an `<input>` / `<textarea>` focuses it; a click on a `<label>`
    // delegates to its associated control (`for` or a wrapped element). Any
    // other click blurs the previously focused field.
    let click_node = resolve_click_node(arena, clicked.map(|r| r.node_id));
    let clicked_input = click_node.is_some_and(|id| is_text_field(arena, id));
    if clicked_input {
        if let Some(node_id) = click_node {
            if is_toggle_input(arena, node_id) {
                // A checkbox / radio toggle: no text focus, but clicking it
                // still blurs any previously focused text field.
                if state.focused_input.is_some() {
                    blur_input(state, msgs, cmds);
                }
                msgs.push(ScriptMsg::ToggleCheck { node_id });
            } else {
                focus_input(state, node_id, arena, msgs, cmds);
                // Clicking an `<input list>` opens its autocomplete dropdown
                // (browser behavior: focus shows the suggestions).
                refresh_datalist(state, Some(hit_map), Some(arena), cmds);
            }
        }
    } else if state.focused_input.is_some() {
        blur_input(state, msgs, cmds);
    }
}

/// `true` for a form control carrying a `disabled` attribute — such
/// controls swallow clicks (no Click dispatch, no focus change).
fn is_disabled_control(arena: &NodeArena, node_id: NodeId) -> bool {
    arena.get(node_id).is_some_and(|n| {
        n.attributes.contains_key("disabled")
            && matches!(
                n.node_type,
                NodeType::Button | NodeType::Input | NodeType::Select | NodeType::Textarea
            )
    })
}

/// Resolve the element a click should act on: a `<label>` hit resolves to
/// its associated control (`for` attribute, or the first wrapped labelable
/// element); every other hit resolves to itself. `None` = no element.
fn resolve_click_node(arena: &NodeArena, clicked: Option<NodeId>) -> Option<NodeId> {
    let id = clicked?;
    if arena.get(id).is_some_and(|n| n.node_type == NodeType::Label) {
        Some(label_control(arena, id).unwrap_or(id))
    } else {
        Some(id)
    }
}

/// The control a `<label>` activates: the element named by `for` (when it is
/// labelable), else the first labelable descendant (input / select /
/// textarea / button) in document order.
fn label_control(arena: &NodeArena, label_id: NodeId) -> Option<NodeId> {
    let label = &arena[label_id];
    if let Some(for_id) = label.attributes.get("for") {
        return arena.iter().find_map(|(id, n)| {
            if n.get_id().is_some_and(|i| i == for_id) && is_labelable(&n.node_type) {
                Some(id)
            } else {
                None
            }
        });
    }
    let mut stack: Vec<NodeId> = label.children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        let node = &arena[id];
        if is_labelable(&node.node_type) {
            return Some(id);
        }
        stack.extend(node.children.iter().rev().copied());
    }
    None
}

/// `true` for elements a `<label>` can activate.
fn is_labelable(node_type: &NodeType) -> bool {
    matches!(
        node_type,
        NodeType::Input
            | NodeType::Select
            | NodeType::Textarea
            | NodeType::Button
            | NodeType::Output
            | NodeType::Progress
            | NodeType::Meter
    )
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

/// Resolve a dropdown row click to an option index.
///
/// Option rows map to themselves; `<optgroup>` header rows map to the first
/// option of that group — a terminal convenience so every visible row is
/// actionable (the browser would ignore a header click). `None` when the row
/// is out of range.
fn row_option(rows: &[SelectRow], row: usize) -> Option<usize> {
    match rows.get(row)? {
        SelectRow::Item(i) => Some(*i),
        SelectRow::Group(_) => rows[row..]
            .iter()
            .find_map(|r| match r {
                SelectRow::Item(i) => Some(*i),
                SelectRow::Group(_) => None,
            }),
    }
}

/// Open the dropdown overlay for `node_id`: resolve the options (a
/// `<select>`'s children, or an `<input list>`'s `<datalist>` filtered by
/// `filter`), the rows and overlay geometry from the DOM, remember them on the
/// input loop, and ask the render thread to paint the overlay.
fn open_overlay(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    node_id: NodeId,
    kind: OverlayKind,
    filter: Option<&str>,
    cmds: &mut Vec<RenderCmd>,
) {
    let Some(region) = hit_map.iter().find(|r| r.node_id == node_id) else {
        return;
    };
    let options = overlay_options(arena, node_id, filter);
    let rows = select_rows(&options);
    let selected = match kind {
        OverlayKind::Select => {
            let value = arena
                .get(node_id)
                .and_then(|n| n.attributes.get("value"))
                .cloned()
                .unwrap_or_default();
            options.iter().position(|o| o.value == value).unwrap_or(0)
        }
        OverlayKind::Datalist => 0,
    };
    let rect = select_overlay_rect(&options, &rows, region.rect);
    state.open_select = Some(SelectOpenState {
        node_id,
        kind,
        rect,
        options,
        rows,
        selected,
        highlight: selected,
    });
    cmds.push(RenderCmd::SelectOpen {
        node_id,
        filter: filter.map(str::to_string),
    });
}

/// Open the `<select>` dropdown for `node_id` (see [`open_overlay`]).
fn open_select(
    state: &mut InputState,
    hit_map: &HitMap,
    arena: &NodeArena,
    node_id: NodeId,
    cmds: &mut Vec<RenderCmd>,
) {
    open_overlay(state, hit_map, arena, node_id, OverlayKind::Select, None, cmds);
}

/// Open / refresh the autocomplete dropdown of a focused `<input list>`.
///
/// The overlay tracks the field's current text: an empty value shows every
/// `<datalist>` suggestion (the browser's focus behavior), otherwise the
/// options are filtered by the current text (case-insensitive substring).
/// `hit_map` / `arena` being `None` (tests / hosts that disable interaction)
/// skips the overlay entirely.
fn refresh_datalist(
    state: &mut InputState,
    hit_map: Option<&HitMap>,
    arena: Option<&NodeArena>,
    cmds: &mut Vec<RenderCmd>,
) {
    let Some(focused) = state.focused_input else {
        return;
    };
    let (Some(hit_map), Some(arena)) = (hit_map, arena) else {
        return;
    };
    let has_list = arena
        .get(focused)
        .is_some_and(|n| n.node_type == NodeType::Input && n.attributes.contains_key("list"));
    if !has_list {
        if let Some(open) = state.open_select.take() {
            cmds.push(RenderCmd::SelectClose {
                node_id: open.node_id,
            });
        }
        return;
    }
    let filter = state.input_value.clone();
    open_overlay(
        state,
        hit_map,
        arena,
        focused,
        OverlayKind::Datalist,
        Some(&filter),
        cmds,
    );
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
    if let Some(idx) = pick && idx < open.options.len() {
            let value = open.options[idx].value.clone();
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

    // An open dropdown takes priority: a press on an option row picks it, a
    // press on an `<optgroup>` header does nothing (browsers keep the
    // dropdown open), a press on the field toggles it closed, any other press
    // closes it and then falls through to the normal dispatch.
    if let Some(open) = state.open_select.clone() {
        if let Some(row) = hit_select_option(&open, x, y) {
            // Option rows pick; `<optgroup>` headers pick their first option
            // (see [`row_option`]) — every visible row is actionable.
            if let Some(option_idx) = row_option(&open.rows, row) {
                close_select(state, arena, Some(option_idx), msgs, cmds);
            }
            return;
        }
        let clicked_self = clicked.is_some_and(|r| r.node_id == open.node_id);
        close_select(state, arena, None, msgs, cmds);
        if clicked_self {
            return;
        }
    }

    // A click on a `<label>` delegates to its associated control; the caret /
    // drag-selection logic below only applies when the pointer is directly
    // over the field itself.
    let click_node = resolve_click_node(arena, clicked.map(|r| r.node_id));
    let direct_field_hit = clicked.is_some_and(|r| click_node == Some(r.node_id));
    let clicked_input = click_node.is_some_and(|id| is_text_field(arena, id));
    match button {
        MouseButton::Left => {
            if clicked_input {
                if let Some(node_id) = click_node {
                    if is_toggle_input(arena, node_id) {
                        // Checkbox / radio: the click handler performs the
                        // toggle — no text focus or drag selection here.
                    } else {
                        focus_input(state, node_id, arena, msgs, cmds);
                        // Pressing an `<input list>` opens its autocomplete
                        // dropdown (browser behavior: focus shows the
                        // suggestions).
                        refresh_datalist(state, Some(hit_map), Some(arena), cmds);
                        if direct_field_hit {
                            state.drag_selecting = true;
                            // Position the caret (and the selection anchor) at
                            // the press point within the field.
                            if let Some(region) = clicked {
                                let idx = if is_textarea_node(arena, node_id) {
                                    textarea_index_at(state, region.rect, x, y)
                                } else {
                                    let inner_x = region.rect.x.saturating_add(1);
                                    let inner_w = region.rect.width.saturating_sub(2);
                                    char_index_at_x(
                                        &state.input_value,
                                        state.input_cursor,
                                        inner_w,
                                        (x as usize).saturating_sub(inner_x),
                                    )
                                };
                                state.input_sel_anchor = Some(idx);
                                state.input_cursor = idx;
                                push_input_render(state, node_id, cmds);
                            }
                        }
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
    // An open dropdown follows the pointer: hovering an option row moves the
    // highlight (group headers are skipped; the highlight stays put over them),
    // repainting only the overlay. This runs even while the button is held
    // (browser select: press-drag over the options) and short-circuits when
    // the highlight is unchanged.
    if let Some(row) = state
        .open_select
        .as_ref()
        .and_then(|open| hit_select_option(open, x, y))
    &&  let Some(idx) = state
            .open_select
            .as_ref()
            .and_then(|open| row_option(&open.rows, row))
    &&  state.open_select.as_ref().is_some_and(|o| o.highlight != idx)
    &&  let Some(open) = &mut state.open_select {
            open.highlight = idx;
            cmds.push(RenderCmd::SelectMove {
                node_id: open.node_id,
                highlight: idx,
            });
    }

    if state.pressed.is_some() {
        // Dragging: the pointer is inside the field it started in, so the
        // selection stays within that field's rectangle.
        if state.drag_selecting {
            if let Some(focused) = state.focused_input
                && let Some(region) = hit_map.iter().find(|r| r.node_id == focused)
            {
                let idx = if state.focused_is_textarea {
                    textarea_index_at(state, region.rect, x, y)
                } else {
                    let inner_x = region.rect.x.saturating_add(1);
                    let inner_w = region.rect.width.saturating_sub(2);
                    char_index_at_x(
                        &state.input_value,
                        state.input_cursor,
                        inner_w,
                        (x as usize).saturating_sub(inner_x),
                    )
                };
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
                // Key releases only matter to the script layer (`keyup`); most
                // terminals never report them, so the built-in shortcuts stay
                // idle here.
                if arena.is_some() {
                    msgs.push(ScriptMsg::KeyUp {
                        target: state.focused_input,
                        key: key_event_info(&key),
                    });
                }
                return cmds;
            }

            // Every press / auto-repeat also reaches the script layer as
            // `keydown` (targeted at the focused `<input>`, or the document
            // `<body>` when nothing is focused). `key_id` correlates the
            // response back so a page `preventDefault()` can cancel the key's
            // built-in shortcut (scroll / zoom / quit, gated below).
            if arena.is_some() {
                state.next_key_id = state.next_key_id.wrapping_add(1);
                msgs.push(ScriptMsg::KeyDown {
                    key_id: state.next_key_id,
                    target: state.focused_input,
                    key: key_event_info(&key),
                });
            }

            // An open dropdown overlay consumes navigation keys: ↑ / ↓ move
            // the highlight (skipping `<optgroup>` headers), `Enter` picks the
            // highlighted option (a `<select>` commits it, an `<input list>`
            // fills the field), `Esc` closes. A plain `<select>` owns the
            // keyboard while open — any other key closes it and is swallowed.
            // A datalist autocomplete instead lets the key edit the field
            // (fall through to the input block, which re-filters).
            if state.open_select.is_some() {
                let Some(arena) = arena else {
                    return cmds;
                };
                let node_id = state
                    .open_select
                    .as_ref()
                    .map(|o| o.node_id)
                    .unwrap();
                let kind = state.open_select.as_ref().map(|o| o.kind).unwrap();
                match key.code {
                    KeyCode::Up | KeyCode::Down => {
                        let dir: i32 = if key.code == KeyCode::Up { -1 } else { 1 };
                        if let Some(open) = &mut state.open_select {
                            let items = select_item_indices(&open.rows);
                            if !items.is_empty() {
                                let pos = items
                                    .iter()
                                    .position(|&i| i == open.highlight)
                                    .unwrap_or(0);
                                let next = items
                                    [(((pos as i32) + dir).rem_euclid(items.len() as i32)) as usize];
                                open.highlight = next;
                                cmds.push(RenderCmd::SelectMove {
                                    node_id,
                                    highlight: next,
                                });
                            }
                        }
                        return cmds;
                    }
                    KeyCode::Enter => {
                        if kind == OverlayKind::Select {
                            let pick = state.open_select.as_ref().map(|o| o.highlight);
                            close_select(state, arena, pick, msgs, &mut cmds);
                        } else {
                            // Datalist: fill the field with the highlighted
                            // suggestion and keep focus (the value commits on
                            // blur / Enter as usual).
                            let pick_value = state
                                .open_select
                                .as_ref()
                                .and_then(|o| o.options.get(o.highlight).map(|opt| opt.value.clone()));
                            if let Some(open) = state.open_select.take() {
                                cmds.push(RenderCmd::SelectClose {
                                    node_id: open.node_id,
                                });
                            }
                            if let Some(value) = pick_value {
                                state.input_value = value;
                                state.input_cursor = state.input_value.chars().count();
                                state.input_sel_anchor = None;
                                push_input_render(state, node_id, &mut cmds);
                                msgs.push(ScriptMsg::InputKeystroke { node_id });
                            }
                        }
                        return cmds;
                    }
                    KeyCode::Esc => {
                        close_select(state, arena, None, msgs, &mut cmds);
                        return cmds;
                    }
                    _ => {
                        close_select(state, arena, None, msgs, &mut cmds);
                        if kind == OverlayKind::Select {
                            return cmds;
                        }
                    }
                }
            }

            // A focused `<input>` consumes the keys it owns: text editing and
            // caret movement. `Esc` blurs it and `Enter` confirms (blur +
            // commit) — both write the value back to the DOM. A focused
            // `<textarea>` owns `Enter` (newline) and `↑`/`↓` (caret) too.
            // Keys the field does *not* own (page navigation such as
            // PageDown / PageUp) fall through to the page-level handler
            // below, matching a browser where an input only intercepts the
            // keys it edits.
            if state.focused_input.is_some() {
                if key.code == KeyCode::Esc || (key.code == KeyCode::Enter && !state.focused_is_textarea)
                {
                    blur_input(state, msgs, &mut cmds);
                    return cmds;
                }
                let Some(focused) = state.focused_input else {
                    return cmds;
                };
                let content_width =
                    textarea_content_width(arena, focused);

                // Word-level editing: Ctrl / Alt + ←/→/Home/End/Backspace/
                // Delete, with optional Shift extending the selection. Must
                // run before the plain Ctrl shortcuts so Ctrl+Left is a word
                // move rather than an ignored combo.
                if (key.modifiers.contains(event::KeyModifiers::CONTROL)
                    || key.modifiers.contains(event::KeyModifiers::ALT))
                    && matches!(
                        key.code,
                        KeyCode::Left
                            | KeyCode::Right
                            | KeyCode::Home
                            | KeyCode::End
                            | KeyCode::Backspace
                            | KeyCode::Delete
                    )
                {
                    if apply_input_word_key(
                        &mut state.input_value,
                        &mut state.input_cursor,
                        &mut state.input_sel_anchor,
                        key.code,
                        key.modifiers,
                    ) {
                        push_input_render(state, focused, &mut cmds);
                        if arena.is_some() {
                            msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
                        }
                        refresh_datalist(state, hit_map, arena, &mut cmds);
                    }
                    return cmds;
                }

                // Ctrl shortcuts: select-all / copy / cut / paste. Other Ctrl
                // combos are ignored (not inserted as characters — browser
                // behaviour).
                if key.modifiers.contains(event::KeyModifiers::CONTROL) {
                    if apply_ctrl_shortcut(state, key.code) {
                        push_input_render(state, focused, &mut cmds);
                        if arena.is_some() {
                            msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
                        }
                        refresh_datalist(state, hit_map, arena, &mut cmds);
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

                // `<textarea>`-owned keys: Enter inserts a newline, ↑ / ↓
                // move the caret between visual lines (keeping the column).
                if state.focused_is_textarea
                    && matches!(
                        key.code,
                        KeyCode::Enter | KeyCode::Up | KeyCode::Down
                    )
                {
                    if apply_textarea_key(
                        &mut state.input_value,
                        &mut state.input_cursor,
                        content_width,
                        key.code,
                    ) {
                        state.input_sel_anchor = None;
                        push_input_render(state, focused, &mut cmds);
                        if arena.is_some() {
                            msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
                        }
                    }
                    return cmds;
                }

                // Field-owned keys stay in the field even when they are
                // boundary no-ops (Home at the start must not scroll the
                // page). Everything else belongs to the page-level handler.
                if is_input_field_key(key.code) {
                    // Plain editing keys clear the selection.
                    let changed = if state.focused_is_textarea {
                        apply_textarea_key(
                            &mut state.input_value,
                            &mut state.input_cursor,
                            content_width,
                            key.code,
                        )
                    } else {
                        apply_input_key(
                            &mut state.input_value,
                            &mut state.input_cursor,
                            key.code,
                        )
                    };
                    if changed {
                        state.input_sel_anchor = None;
                        push_input_render(state, focused, &mut cmds);
                        if arena.is_some() {
                            msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
                        }
                        refresh_datalist(state, hit_map, arena, &mut cmds);
                    }
                    return cmds;
                }
                // Fall through to the page-level key handling below.
            }

            // Page-level built-ins: no dropdown / field consumed the key. With
            // the script layer active, scroll / zoom / quit keys are deferred
            // pending the `keydown` gate — a page `preventDefault()` cancels
            // them, a gate timeout (busy or blocked script thread) falls back
            // to running them so input stays responsive. `Tab` focus cycling
            // is input-control behaviour and always runs immediately.
            let builtin = page_level_builtins(state, key, rows, max_scroll, arena, msgs);
            if arena.is_some()
                && !builtin.is_empty()
                && !matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
            {
                state.key_gate = Some(KeyGate {
                    id: state.next_key_id,
                    cmds: builtin,
                });
                return cmds;
            }
            cmds.extend(builtin);
            cmds
        }

        Ok(Event::Mouse(mouse)) => {
            let ctrl = mouse.modifiers.contains(event::KeyModifiers::CONTROL);
            state.last_mouse = Some((mouse.column, mouse.row));
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
                        // Press-drag-release over a dropdown row picks it
                        // (the browser's native `<select>` interaction), even
                        // when the release lands off the press cell. Rows map
                        // to option indices like the mousedown pick above:
                        // `<optgroup>` headers pick their first option.
                        if let Some(open) = state.open_select.clone()
                            && let Some(row) = hit_select_option(&open, x, y)
                            && let Some(option_idx) = row_option(&open.rows, row)
                        {
                                close_select(state, arena, Some(option_idx), msgs, &mut cmds);
                                return cmds;
                            
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
                // must reach the drag / hover handler. A `Moved` event also
                // proves no button is held: a press whose release was lost
                // (the console can drop the `Up` during a resize) is dropped
                // here, otherwise every later move is misread as a drag and
                // hover / click events stay silent forever.
                MouseEventKind::Drag(_) | MouseEventKind::Moved => {
                    if let (Some(hit_map), Some(_)) = (hit_map, arena) {
                        if !matches!(mouse.kind, MouseEventKind::Drag(_)) {
                            state.drop_stale_press();
                        }
                        let (x, y) = canvas_pos(state, mouse.column, mouse.row);
                        handle_mouse_move(state, hit_map, x, y, msgs, &mut cmds);
                    }
                }
                _ => {}
            }
            cmds
        }

        Ok(Event::Resize(_, _new_rows)) => {
            // A terminal resize on Windows can reset the console input mode
            // (mouse capture off, keyboard line-buffered); re-apply it right
            // away. The run loop also polls the size every iteration as a
            // fallback for resize paths that emit no event.
            reapply_console_modes();
            // A resize invalidates every canvas coordinate the input loop
            // caches (a stuck press would kill hover / click, a stale
            // selection or dropdown would paint wrong cells), and the hover
            // target must be re-validated against the rebuilt hit map.
            state.invalidate_viewport(&mut cmds);
            state.request_resize_revalidation();
            // The render thread re-lays-out against the new size and clamps
            // its own scroll position; mirror the clamp here (to the last
            // known ceiling) so the scroll sent with the redraw matches, and
            // the run loop re-syncs to the freshly computed ceiling on the
            // next iteration.
            state.clamp_scroll_to(max_scroll);
            cmds.push(RenderCmd::Redraw {
                scroll_y: state.scroll_y,
                zoom: state.zoom,
            });
            cmds
        }

        Ok(Event::Paste(pasted)) => {
            // Bracketed paste (enabled at startup): the terminal delivers the
            // system clipboard verbatim. Insert it at the caret of the focused
            // field — `<textarea>` keeps newlines, single-line fields collapse
            // them — then fire `input` and repaint. Nothing to do when no
            // field is focused (the terminal already showed the pasted text).
            if let Some(focused) = state.focused_input {
                if paste_into_field(state, &pasted, focused, &mut cmds) && arena.is_some() {
                    msgs.push(ScriptMsg::InputKeystroke { node_id: focused });
                }
            }
            cmds
        }

        _ => cmds,
    }
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
    state.input_cursor = state
        .input_cursor
        .saturating_add(text.chars().count());
    state.input_sel_anchor = None;
    push_input_render(state, focused, cmds);
    true
}

/// The page-level built-in action for a key that no `<select>` dropdown or
/// focused `<input>` consumed: `Ctrl` zoom / document jumps, page text-
/// selection clearing, `Shift`+Space page-up, plain Space / PageUp / PageDown /
/// arrows / `g` / `G` / `j` / `k` scrolling, `Home` / `End`, `q` / `Esc` quit
/// and `Tab` / `Shift`+`Tab` focus cycling.
///
/// Returns the render commands the key produces (empty = no built-in action,
/// e.g. a scroll key already at the page boundary). With the script layer
/// active the caller defers scroll / zoom / quit commands until the keydown
/// gate verdict; `Tab` focus cycling pushes script messages, so it is excluded
/// from the gate and always runs immediately.
fn page_level_builtins(
    state: &mut InputState,
    key: KeyEvent,
    rows: u16,
    max_scroll: u32,
    arena: Option<&NodeArena>,
    msgs: &mut Vec<ScriptMsg>,
) -> Vec<RenderCmd> {
    let mut cmds = Vec::new();
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
            // Copy a page text selection (inside an `<input>` the Ctrl
            // shortcuts are handled earlier). `Ctrl+X` on read-only page text
            // degrades to a copy, like a browser.
            KeyCode::Char('c') | KeyCode::Char('x') => {
                if state.focused_input.is_none() && state.text_sel_anchor.is_some() && 
                    let Ok(guard) = state.selection_text.lock() {
                        state.input_clipboard = guard.clone();
                    
                }
            }
            // `Ctrl` + Home / End jump to the top / bottom of the page
            // (matching most editors' document navigation).
            KeyCode::Home => {
                if state.scroll_y == 0 {
                    return cmds;
                }
                state.scroll_y = 0;
                return vec![RenderCmd::Rerender {
                    scroll_y: state.scroll_y,
                }];
            }
            KeyCode::End => {
                if state.scroll_y >= max_scroll {
                    return cmds;
                }
                state.scroll_y = max_scroll;
                return vec![RenderCmd::Rerender {
                    scroll_y: state.scroll_y,
                }];
            }
            _ => {}
        }
    } else if state.text_sel_anchor.is_some() {
        // A non-Ctrl key abandons the page text selection (the selection is
        // a pointer-driven, transient visual state).
        clear_text_selection(state, &mut cmds);
    }

    // `Shift` + Space scrolls up a page (the browser flip of the plain
    // `Space` page-down handled below).
    if key.code == KeyCode::Char(' ') && key.modifiers.contains(event::KeyModifiers::SHIFT) {
        if state.scroll_y == 0 {
            return cmds;
        }
        let step = (rows as u32).saturating_sub(1).max(1);
        state.scroll_y = state.scroll_y.saturating_sub(step);
        return vec![RenderCmd::Rerender {
            scroll_y: state.scroll_y,
        }];
    }

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => cmds.push(RenderCmd::Quit),

        // vim-style document jumps: `g` to the top, `G` to the bottom
        // (shifted `g` — the shift modifier is read via the `Char('G')` code
        // crossterm reports).
        KeyCode::Char('g') => {
            if state.scroll_y == 0 {
                return cmds;
            }
            state.scroll_y = 0;
            cmds.push(RenderCmd::Rerender {
                scroll_y: state.scroll_y,
            });
        }
        KeyCode::Char('G') => {
            if state.scroll_y >= max_scroll {
                return cmds;
            }
            state.scroll_y = max_scroll;
            cmds.push(RenderCmd::Rerender {
                scroll_y: state.scroll_y,
            });
        }
        // Plain `Space` scrolls down a page, like a browser.
        KeyCode::Char(' ') => {
            if state.scroll_y >= max_scroll {
                return cmds;
            }
            let step = (rows as u32).saturating_sub(1).max(1);
            state.scroll_y = state.scroll_y.saturating_add(step);
            cmds.push(RenderCmd::Rerender {
                scroll_y: state.scroll_y,
            });
        }
        // `Tab` / `Shift`+`Tab` cycle keyboard focus between the page's
        // `<input>` fields (browser tab order). With no field focused, `Tab`
        // focuses the first one.
        KeyCode::Tab => {
            if let Some(arena) = arena {
                let ids = collect_input_ids(arena);
                if !ids.is_empty() {
                    let next = match state
                        .focused_input
                        .and_then(|cur| ids.iter().position(|&id| id == cur))
                    {
                        Some(p) => ids[(p + 1) % ids.len()],
                        None => ids[0],
                    };
                    focus_input(state, next, arena, msgs, &mut cmds);
                }
            }
        }
        KeyCode::BackTab => {
            if let Some(arena) = arena {
                let ids = collect_input_ids(arena);
                if !ids.is_empty() {
                    let next = match state
                        .focused_input
                        .and_then(|cur| ids.iter().position(|&id| id == cur))
                    {
                        Some(p) => ids[(p + ids.len() - 1) % ids.len()],
                        None => ids[ids.len() - 1],
                    };
                    focus_input(state, next, arena, msgs, &mut cmds);
                }
            }
        }

        KeyCode::Up | KeyCode::Char('k') => {
            // Already at the top boundary — drop the event instead of
            // forwarding a no-op scroll to the render thread.
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
            // Use the actual upper bound rather than u32::MAX so the render
            // thread receives a value it doesn't have to clamp.
            state.scroll_y = max_scroll;
            cmds.push(RenderCmd::Rerender {
                scroll_y: state.scroll_y,
            });
        }
        _ => {}
    }
    cmds
}

/// Convert a screen position to canvas coordinates by adding the scroll
/// offset. Zoom is not yet wired into layout, so no zoom scaling is applied.
fn canvas_pos(state: &InputState, column: u16, row: u16) -> (u32, u32) {
    (column as u32, row as u32 + state.scroll_y)
}

/// Re-apply the console input mode after a terminal resize.
///
/// On Windows, resizing the console / ConPTY buffer can reset the console's
/// input mode: mouse capture is dropped (`ENABLE_MOUSE_INPUT` cleared, quick-
/// edit selection re-engages) and the keyboard falls back to cooked mode, so
/// input appears completely dead until the app re-issues raw mode + mouse
/// capture. Both calls are idempotent, so this is safe to run on every loop
/// iteration. On other platforms a resize never touches the terminal modes, so
/// this is a no-op.
#[cfg(windows)]
pub(crate) fn reapply_console_modes() {
    if let Err(e) = crossterm::terminal::enable_raw_mode() {
        log::error!("Failed to re-enable raw mode after resize: {}", e);
    }
    // Pass a *sink* writer: `EnableMouseCapture` is a WinAPI call (an
    // input-handle `SetConsoleMode`) on Windows, but `execute!` flushes the
    // passed writer first to preserve command ordering. Flushing
    // `std::io::stdout()` would take the global stdout lock — which the
    // render thread can hold while blocked on a slow console write — and a
    // fast wheel scroll (large diffs every frame) makes that stall long
    // enough to look like the whole app froze. A sink flushes nothing and
    // still runs the WinAPI path. Bracketed paste is re-sent too so a
    // terminal reset does not silently drop it.
    if let Err(e) = crossterm::execute!(
        std::io::sink(),
        crossterm::event::EnableMouseCapture,
        crossterm::event::EnableBracketedPaste
    ) {
        log::error!("Failed to re-enable input modes after resize: {}", e);
    }
}

/// Non-Windows resize does not touch the terminal modes; nothing to re-apply.
#[cfg(not(windows))]
pub(crate) fn reapply_console_modes() {}
