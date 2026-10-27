use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use boa_engine::{Context, JsError};

use crate::{
    events::HtmlEvent,
    resources::AsyncResult,
    source::SharedSourceText,
    tdom::node::{NodeArena, NodeId},
    tui::graph_api::{DirtyInfo, RenderCmd},
};

use super::host::HostFn;

/// Per-session engine state shared between the Boa natives and the Rust side.
///
/// Held as `Rc<RefCell<…>>` inside a `thread_local` so every native (a plain
/// `Copy` closure / fn pointer) can reach it without `unsafe` captures.
pub(crate) struct ScriptState {
    /// The live DOM scripts mutate.
    pub arena: NodeArena,
    pub root_id: NodeId,
    /// CSS extracted from the document currently owned by this engine.
    ///
    /// A whole-document reload changes this source even when JS has not
    /// touched the new arena yet; the script reply carries it to the render
    /// thread so `<style>` edits can take effect.
    pub document_css: SharedSourceText,
    /// Whether the engine replaced its document since the last reply.
    ///
    /// Set by `ScriptEngine::reload` and consumed by the script-thread bridge;
    /// ordinary DOM mutations leave it false.
    pub document_reload: bool,
    /// Session-level JS supplied beside the HTML source. It runs after the
    /// document's inline `<script>` blocks and is re-run after whole-document
    /// reloads; `ReloadJs` can replace it without replacing the DOM.
    pub session_js: SharedSourceText,
    /// Render commands pushed by host functions during JS execution.
    pub render_cmds: Vec<RenderCmd>,
    /// Nodes whose content or attributes JS changed since the last render.
    /// Non-empty means the DOM needs a re-render.
    pub dirty: DirtyInfo,
    /// The event currently being dispatched (for host functions).
    pub current_event: Option<HtmlEvent>,
    /// Registered host functions, keyed by JS global name.
    pub host_fns: HashMap<String, HostFn>,
    /// Pending `setTimeout` / `setInterval` registrations, driven by the
    /// script thread's message loop (the only thread allowed to run JS).
    pub timers: Vec<TimerEntry>,
    /// Monotonic id source for timer registrations.
    pub next_timer_id: u32,
    /// In-flight `fetch` ids. The promise's `resolve` / `reject` functions
    /// live in the GC-rooted `__htuiFetchResolvers` JS registry (storing them
    /// in Rust would leave them invisible to Boa's collector); this set only
    /// tracks which fetches are outstanding so a stale completion can be
    /// detected.
    pub active_fetches: std::collections::HashSet<u32>,
    /// Monotonic id source for `fetch` registrations.
    pub next_fetch_id: u32,
    /// Receive end of the load channel (see [`resources::load_async`]).
    pub fetch_rx: Receiver<AsyncResult>,
    /// Send end handed to [`resources::load_async`] when a `fetch` starts.
    pub fetch_tx: Sender<AsyncResult>,
}

impl ScriptState {
    /// Create the fetch channel pair. `load_async` gets a clone of `tx`; the
    /// script thread owns `rx` and drains it on every loop iteration.
    pub(super) fn fetch_channel() -> (Sender<AsyncResult>, Receiver<AsyncResult>) {
        mpsc::channel()
    }
}

/// A pending `setTimeout` / `setInterval` registration.
pub(crate) struct TimerEntry {
    pub id: u32,
    /// When the callback is due (monotonic clock).
    pub deadline: Instant,
    /// Repeat cadence for `setInterval` (re-scheduling after each fire);
    /// ignored for one-shot `setTimeout` entries.
    pub period: Duration,
    /// `true` for `setInterval`, `false` for `setTimeout`.
    pub repeat: bool,
}

thread_local! {
    /// The active engine state for the current thread (the input loop).
    pub(crate) static ENGINE_STATE: RefCell<Option<Rc<RefCell<ScriptState>>>> = const {RefCell::new(None)} ;
    /// The terminal UI state shared with the input loop (set on the script
    /// thread by `run_script_thread`). Natives read it for
    /// `window.innerWidth` / `innerHeight` / `scrollY`; unset (direct
    /// `ScriptEngine::new` in tests) reads as zeros.
    static UI_STATE: RefCell<Option<std::sync::Arc<SharedUiState>>> = const { RefCell::new(None) };
}

/// Terminal UI state the input loop owns but JS needs to read: the viewport
/// in cells and the page scroll offset. Shared as `Arc` (the input loop and
/// the script thread hold clones) and updated with relaxed atomics — the
/// values are advisory for scripts, never used for synchronization.
#[derive(Debug, Default)]
pub struct SharedUiState {
    pub cols: AtomicU32,
    pub rows: AtomicU32,
    pub scroll_y: AtomicU32,
}

impl SharedUiState {
    /// Snapshot the three values in one read.
    pub fn load(&self) -> (u32, u32, u32) {
        (
            self.cols.load(Ordering::Relaxed),
            self.rows.load(Ordering::Relaxed),
            self.scroll_y.load(Ordering::Relaxed),
        )
    }
}

/// Publish the UI state for the current (script) thread; natives read it via
/// [`with_ui`].
pub(crate) fn set_ui_state(ui: std::sync::Arc<SharedUiState>) {
    UI_STATE.with(|cell| *cell.borrow_mut() = Some(ui));
}

/// The UI state snapshot for `window.innerWidth` & co. `None` when no input
/// loop is attached (a bare `ScriptEngine::new` in tests).
pub(crate) fn with_ui() -> Option<(u32, u32, u32)> {
    UI_STATE.with(|cell| cell.borrow().as_ref().map(|ui| ui.load()))
}

/// Engine pieces produced by a successful build: the Boa context and the
/// script state it operates on.
pub(super) type EngineParts = (Context, Rc<RefCell<ScriptState>>);

/// Build-failure payload: the error plus the host-function map so a failed
/// reload can restore the previous session.
pub(super) type EngineBuildError = (JsError, HashMap<String, HostFn>);
