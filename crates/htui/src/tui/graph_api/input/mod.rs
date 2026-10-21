use std::sync::{Arc, Mutex};

use compact_str::CompactString;

use crate::tdom::node::NodeId;

mod cmd;
mod edit;
mod field;
mod focus;
mod key_info;
mod mouse;
mod page;
mod refresh;
mod select;

pub use cmd::{DirtyInfo, DrainedBatch, RenderCmd, SelectionMode};
pub use edit::{
    apply_input_key, apply_input_word_key, is_input_field_key, next_word_boundary,
    paste_into_field, prev_word_boundary, selection_range,
};
pub use field::{
    KEY_GATE_TIMEOUT, KeyGate, apply_textarea_key, collect_input_ids, number_input_allows_char,
};
pub use focus::apply_focus_requests;
pub use key_info::key_event_info;
pub use mouse::{handle_click, handle_mouse_down, handle_mouse_move, handle_mouse_up};
pub use refresh::trigger_refresh;
pub use select::{OverlayKind, SelectOpenState};

use field::clear_text_selection;

const SCROLL_LINES_PER_TICK: u32 = 3;
const ZOOM_STEP: f32 = 0.1;
const ZOOM_MIN: f32 = 0.2;
const ZOOM_MAX: f32 = 5.0;

/// Minimum pointer travel (Chebyshev distance in cells) before a press turns
/// into a page text selection drag. A press alone never selects, and a
/// jittery click — press, one-cell wiggle, release — must not leave a single
/// highlighted character behind (browsers use a similar threshold).
const TEXT_SEL_DRAG_THRESHOLD: u32 = 2;

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
    pub input_value: CompactString,
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
    /// Whether the current press armed a page text selection — the mode above
    /// was set by a press whose copy feature is enabled. Without this, a
    /// disabled feature's press would leave a stale mode behind and a later
    /// drag could still start a selection in the wrong shape.
    text_sel_armed: bool,
    /// Whether the flow (left-drag) page text selection + copy is enabled.
    /// Seeded from [`Config`](crate::config::Config); when `false` a left drag
    /// over page content never starts a selection (and `Ctrl+C` copies
    /// nothing).
    pub copy_flow: bool,
    /// Whether the block (right-drag) rectangle selection + copy is enabled.
    /// Seeded from [`Config`](crate::config::Config); when `false` a right
    /// drag never starts a rectangle selection (and `Ctrl+C` copies nothing).
    pub copy_block: bool,
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
    /// Create the default input state (top of page, 100% zoom, no focus or selection).
    pub fn new() -> Self {
        Self {
            scroll_y: 0,
            zoom: 1.0,
            pressed: None,
            focused_input: None,
            focused_is_textarea: false,
            input_value: CompactString::new(""),
            input_cursor: 0,
            input_sel_anchor: None,
            input_clipboard: String::new(),
            drag_selecting: false,
            last_mouse: None,
            resize_pending: false,
            text_sel_anchor: None,
            text_sel_end: None,
            text_sel_mode: SelectionMode::Flow,
            text_sel_armed: false,
            copy_flow: true,
            copy_block: true,
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
