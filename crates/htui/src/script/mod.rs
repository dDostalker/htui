//! JavaScript scripting layer over the Boa engine.
//!
//! # Architecture
//!
//! A [`ScriptEngine`] owns a Boa [`Context`] plus a mutable DOM
//! ([`NodeArena`]) that scripts manipulate through browser-aligned bindings
//! (`document.getElementById`, `el.textContent`, `el.addEventListener`, …).
//!
//! The engine is a *side channel* like the rest of the event system: the
//! render thread still owns layout/paint, but when JS mutates the DOM the
//! engine reports a dirty flag and the input loop forwards the live arena to
//! the render thread via [`RenderCmd::RenderArena`](crate::tui::graph_api::RenderCmd::RenderArena)
//! — no HTML re-parse needed.
//!
//! # Event model
//!
//! `on*` attribute bodies are compiled to `function (event) { body }` at
//! startup (browser semantics: a bare identifier only evaluates, it never
//! calls). `addEventListener(type, fn)` registers JS functions on the JS side
//! (GC-safe). Dispatch walks the hit region's ancestor path target-first,
//! firing each element's listeners with a DOM-style `Event` object
//! (`target`, `currentTarget`, `x`, `y`, `stopPropagation()`).
//!
//! # Host functions
//!
//! Entries of [`EventBindings::host`] become JS globals callable from scripts;
//! they can push [`RenderCmd`]s and return values. See [`host`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use boa_engine::object::builtins::JsFunction;
use boa_engine::{Context, JsError, JsResult, JsValue, NativeFunction, Source, js_string};
use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;

use crate::{
    events::{EventBindings, EventType, HitRegion, HtmlEvent, KeyEventInfo},
    html::html_parse::Parser,
    resources::AsyncResult,
    tdom::node::{Node, NodeArena, NodeId, NodeType},
    tui::graph_api::RenderCmd,
};

mod dom;
mod host;
mod msg;

pub mod convert;

pub use convert::{FromJs, IntoJs, JsObjectBuilder};
pub use host::{HostCall, HostFn};
pub use msg::{ScriptDirty, ScriptEvent, ScriptMsg, ScriptResponse, apply_message, run_script_thread};

use dom::{
    add_listener, get_listeners, install_builtins, node_wrapper, response_object, with_state,
};

use crate::tui::graph_api::DirtyInfo;

/// Per-session engine state shared between the Boa natives and the Rust side.
///
/// Held as `Rc<RefCell<…>>` inside a `thread_local` so every native (a plain
/// `Copy` closure / fn pointer) can reach it without `unsafe` captures.
pub(crate) struct ScriptState {
    /// The live DOM scripts mutate.
    pub arena: NodeArena,
    pub root_id: NodeId,
    /// Render commands pushed by host functions during JS execution.
    pub render_cmds: Vec<RenderCmd>,
    /// Nodes whose content or attributes JS changed since the last render.
    /// Non-empty means the DOM needs a re-render (the old `dom_dirty` flag
    /// merged into this set).
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
    fn fetch_channel() -> (Sender<AsyncResult>, Receiver<AsyncResult>) {
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
}

/// Browser-aligned JS engine attached to a live DOM.
pub struct ScriptEngine {
    context: Context,
    state: Rc<RefCell<ScriptState>>,
}

/// Bootstrap script: the GC-rooted listener / element / timer registries.
const BOOTSTRAP: &str = include_str!("bootstrap.js");

impl ScriptEngine {
    /// Create an engine from an HTML document and the application bindings.
    ///
    /// Parses the document (arena + `<script>` sources), installs the DOM
    /// globals and host functions, compiles `on*` attribute bodies into
    /// listeners, then runs every `<script>` block. Errors are returned as
    /// Boa errors; individual script failures log a warning and continue.
    pub fn new(html: &str, bindings: EventBindings) -> JsResult<Self> {
        let (context, state) = match Self::build_engine(html, bindings.host) {
            Ok(v) => v,
            Err((e, _)) => return Err(e),
        };
        Ok(ScriptEngine { context, state })
    }

    /// Replace the live document with a freshly parsed one and re-run its
    /// scripts, keeping the session's host functions.
    ///
    /// The engine is the single source of truth for the document: a
    /// whole-document replacement re-parses here and marks the root dirty, so
    /// the change reaches the render thread as a `RenderArena` — never as an
    /// independent re-parse on the render side. On failure the previous
    /// document is left intact and the error is returned.
    pub fn reload(&mut self, html: &str) -> JsResult<()> {
        let host_fns = std::mem::take(&mut self.state.borrow_mut().host_fns);
        let (context, state) = match Self::build_engine(html, host_fns) {
            Ok(v) => v,
            Err((e, host_fns)) => {
                // Restore the previous session so a failed reload leaves it
                // usable: put the host map back and re-point the thread-local
                // state the DOM natives read through.
                self.state.borrow_mut().host_fns = host_fns;
                ENGINE_STATE.with(|s| *s.borrow_mut() = Some(Rc::clone(&self.state)));
                return Err(e);
            }
        };
        // The whole document changed: mark the root dirty so the next
        // response carries the fresh arena to the render thread.
        {
            let mut st = state.borrow_mut();
            let root_id = st.root_id;
            st.dirty.nodes.push(root_id);
        }
        self.context = context;
        self.state = state;
        Ok(())
    }

    /// Build a fresh Boa context + DOM state for `html`, consuming `host_fns`.
    ///
    /// On failure the host map is returned inside the `Err` so a caller that
    /// borrowed it (like [`reload`](Self::reload)) can restore the previous
    /// session.
    fn build_engine(
        html: &str,
        host_fns: HashMap<String, HostFn>,
    ) -> Result<(Context, Rc<RefCell<ScriptState>>), (JsError, HashMap<String, HostFn>)> {
        let doc = Parser::new(html).parse_with_scripts();
        let (fetch_tx, fetch_rx) = ScriptState::fetch_channel();
        let state = Rc::new(RefCell::new(ScriptState {
            arena: doc.arena,
            root_id: doc.root_id,
            render_cmds: Vec::new(),
            dirty: DirtyInfo::default(),
            current_event: None,
            host_fns,
            timers: Vec::new(),
            next_timer_id: 0,
            active_fetches: std::collections::HashSet::new(),
            next_fetch_id: 0,
            fetch_rx,
            fetch_tx,
        }));
        ENGINE_STATE.with(|s| *s.borrow_mut() = Some(Rc::clone(&state)));

        let mut context = Context::default();

        // GC-rooted registries used by the DOM natives.
        if let Err(e) = context.eval(Source::from_bytes(BOOTSTRAP.as_bytes())) {
            return Err((e, std::mem::take(&mut state.borrow_mut().host_fns)));
        }
        if let Err(e) = install_builtins(&mut context) {
            return Err((e, std::mem::take(&mut state.borrow_mut().host_fns)));
        }

        // Expose host functions as JS globals.
        let host_names: Vec<String> = state.borrow().host_fns.keys().cloned().collect();
        for name in host_names {
            if let Err(e) = register_host_global(&mut context, &name) {
                return Err((e, std::mem::take(&mut state.borrow_mut().host_fns)));
            }
        }

        // Compile `on*` attribute bodies into listeners.
        if let Err(e) = compile_attribute_listeners(&mut context, &state) {
            return Err((e, std::mem::take(&mut state.borrow_mut().host_fns)));
        }

        // Run the document's `<script>` blocks.
        for script in &doc.scripts {
            if let Err(e) = context.eval(Source::from_bytes(script.as_bytes())) {
                log::warn!("Script execution failed: {e}");
            }
        }

        // Browser `autofocus`: the first focusable element carrying the
        // `autofocus` attribute (document order) requests focus once the
        // input loop processes the command — the same `FocusRequest` path a
        // JS `el.focus()` uses.
        let autofocus = state.borrow().arena.iter().find_map(|(id, n)| {
            (matches!(n.node_type, NodeType::Input | NodeType::Textarea)
                && n.attributes.contains_key("autofocus"))
            .then_some(id)
        });
        if let Some(id) = autofocus {
            state
                .borrow_mut()
                .render_cmds
                .push(RenderCmd::FocusRequest { node_id: id });
        }
        Ok((context, state))
    }

    /// Rewrite `RenderCmd::RenderHtml` pushed by host functions into an
    /// in-engine document reload, so the script engine stays the single
    /// source of truth for the document (see [`reload`](Self::reload)). A
    /// failed reload keeps the command for the render thread's legacy path.
    fn intercept_render_html(&mut self, cmds: Vec<RenderCmd>) -> Vec<RenderCmd> {
        let mut out = Vec::with_capacity(cmds.len());
        for cmd in cmds {
            match cmd {
                RenderCmd::RenderHtml { html } => {
                    if let Err(e) = self.reload(&html) {
                        log::error!("In-engine document reload failed: {e}");
                        out.push(RenderCmd::RenderHtml { html });
                    }
                }
                other => out.push(other),
            }
        }
        out
    }

    /// How long the message loop may block before the earliest pending timer
    /// fires, or `None` when no timer is active (block indefinitely).
    pub fn next_timer_timeout(&self) -> Option<Duration> {
        let st = self.state.borrow();
        st.timers
            .iter()
            .map(|t| t.deadline.saturating_duration_since(Instant::now()))
            .min()
    }

    /// Fire every timer whose deadline has passed, in registration order.
    ///
    /// Callbacks run in the calling thread — the script thread in production —
    /// exactly like a browser event-loop task, so a callback that rewrites the
    /// DOM marks nodes dirty as usual. Returns render commands pushed by host
    /// functions during the dispatch. `setInterval` callbacks are re-scheduled
    /// relative to the fire time; a badly-lagged interval jumps to `now +
    /// period` instead of bursting. `setTimeout` callbacks are one-shot.
    pub fn run_due_timers(&mut self) -> Vec<RenderCmd> {
        let now = Instant::now();
        // Snapshot the due ids first: callbacks may register or clear timers,
        // so the table mutates while we iterate.
        let due: Vec<u32> = {
            let st = self.state.borrow();
            st.timers
                .iter()
                .filter(|t| t.deadline <= now)
                .map(|t| t.id)
                .collect()
        };
        if due.is_empty() {
            return Vec::new();
        }
        // Re-schedule / remove before firing so a slow callback cannot make a
        // missed repeat deadline fire again immediately after.
        {
            let mut st = self.state.borrow_mut();
            st.timers.retain_mut(|t| {
                if t.deadline > now {
                    return true;
                }
                if t.repeat {
                    t.deadline = now + t.period;
                    true
                } else {
                    false
                }
            });
        }
        for id in due {
            let callback = timer_callback(&mut self.context, id);
            if !callback.is_callable() {
                log::debug!("Timer {id} has no callable callback — skipping");
                continue;
            }
            if let Some(f) = callback.to_object(&mut self.context).ok()
                && let Err(e) = f.call(&JsValue::undefined(), &[], &mut self.context)
            {
                log::warn!("Timer callback threw: {e}");
            }
        }
        let mut st = self.state.borrow_mut();
        let cmds = std::mem::take(&mut st.render_cmds);
        drop(st);
        self.intercept_render_html(cmds)
    }

    /// Drain completed loads and settle their `fetch` promises.
    ///
    /// Called on every script-thread loop iteration after message / timer
    /// processing. Each completed [`AsyncResult`] carries a `{id}:{url}` key
    /// (see [`crate::resources::load_async`]); the id looks up the promise's
    /// `resolve` / `reject` functions stashed by `fetch`, which are invoked
    /// with a `Response`-like object (or an error). Promise reaction callbacks
    /// (`.then` / `await`) run through [`Context::run_jobs`], so this is the
    /// microtask half of the engine's event loop.
    pub fn poll_fetches(&mut self) {
        loop {
            let result = self.state.borrow().fetch_rx.try_recv();
            match result {
                Ok(AsyncResult { id, url, outcome }) => {
                    self.settle_fetch(id, &url, outcome);
                }
                Err(e) => {
                    if !matches!(e, std::sync::mpsc::TryRecvError::Empty) {
                        log::debug!("fetch channel error: {e}");
                    }
                    break;
                }
            }
        }
        if let Err(e) = self.context.run_jobs() {
            log::warn!("Microtask queue run failed: {e}");
        }
    }

    /// Resolve or reject the promise for a single completed fetch.
    ///
    /// The resolvers live in the GC-rooted `__htuiFetchResolvers` JS registry
    /// (keyed by fetch id); this method reads them out, settles the promise,
    /// and removes the entry.
    fn settle_fetch(
        &mut self,
        id: u32,
        url: &str,
        outcome: Result<std::sync::Arc<Vec<u8>>, String>,
    ) {
        if !self.state.borrow_mut().active_fetches.remove(&id) {
            log::warn!("fetch result for unknown id {id} (url {url}) — dropped");
            return;
        }
        let (resolve, reject) = match self.take_fetch_resolvers(id) {
            Some(v) => v,
            None => {
                log::warn!("fetch id {id} has no JS resolvers — dropped");
                return;
            }
        };
        let value = match outcome {
            Ok(resource) => response_object(&mut self.context, url.to_string(), resource),
            Err(message) => Err(JsError::from_opaque(
                js_string!(format!("fetch failed for {url}: {message}")).into(),
            )),
        };
        let result = match value {
            Ok(v) => resolve.call(&JsValue::undefined(), &[v], &mut self.context),
            Err(e) => reject.call(
                &JsValue::undefined(),
                &[e.to_opaque(&mut self.context)],
                &mut self.context,
            ),
        };
        if let Err(e) = result {
            log::warn!("fetch promise settlement failed: {e}");
        }
    }

    /// Pull the `[resolve, reject]` array for a fetch id out of the JS
    /// registry and delete the entry (each fetch settles at most once).
    fn take_fetch_resolvers(&mut self, id: u32) -> Option<(JsFunction, JsFunction)> {
        let global = self.context.global_object();
        let registry = global
            .get(js_string!("__htuiFetchResolvers"), &mut self.context)
            .ok()?
            .to_object(&mut self.context)
            .ok()?;
        let key = js_string!(id.to_string());
        let entry = registry.get(key.clone(), &mut self.context).ok()?;
        if entry.is_undefined() {
            return None;
        }
        let obj = entry.to_object(&mut self.context).ok()?;
        let resolve = obj
            .get(js_string!("resolve"), &mut self.context)
            .ok()
            .and_then(|v| v.as_callable())
            .and_then(JsFunction::from_object);
        let reject = obj
            .get(js_string!("reject"), &mut self.context)
            .ok()
            .and_then(|v| v.as_callable())
            .and_then(JsFunction::from_object);
        if let Err(e) = registry.delete_property_or_throw(key, &mut self.context) {
            log::warn!("Failed to remove fetch resolver for id {id}: {e}");
        }
        match (resolve, reject) {
            (Some(r), Some(j)) => Some((r, j)),
            _ => None,
        }
    }

    /// Clone of the live DOM arena.
    pub fn arena(&self) -> NodeArena {
        self.state.borrow().arena.clone()
    }

    /// Root node id of the live DOM.
    pub fn root_id(&self) -> NodeId {
        self.state.borrow().root_id
    }

    /// Whether a JS call mutated the DOM since the last take, and which nodes
    /// were touched. Consumes the accumulated set so each mutation batch is
    /// reported exactly once.
    pub fn take_dirty(&mut self) -> DirtyInfo {
        std::mem::take(&mut self.state.borrow_mut().dirty)
    }

    /// Drain render commands queued *outside* a dispatch — e.g. `el.focus()`
    /// from a top-level `<script>` or a host call inside a settled `fetch`
    /// `.then`. [`dispatch`](Self::fire) only takes the queue for its own
    /// calls, so without an explicit drain a load-time command would sit until
    /// the first event. Applies the same [`RenderHtml`](RenderCmd::RenderHtml)
    /// interception as every other command source.
    pub fn take_pending_cmds(&mut self) -> Vec<RenderCmd> {
        let mut st = self.state.borrow_mut();
        let cmds = std::mem::take(&mut st.render_cmds);
        drop(st);
        self.intercept_render_html(cmds)
    }

    /// Commit an `<input>` field's edited value into the DOM `value`
    /// attribute and mark the node dirty.
    ///
    /// The DOM is the single source of truth for a field's content: editing
    /// while focused repaints through the lightweight `input_states` path, and
    /// this writes the result back on blur / Enter / focus-switch so the next
    /// focus (or any later re-render) sees the edited value. Marking the node
    /// dirty guarantees the commit reaches the render thread as a
    /// `RenderArena`, so a JS blur handler that rewrites the value (e.g.
    /// clearing a submitted box) is what ends up on screen. Committing an
    /// unchanged value is a no-op (no dirty marker), mirroring the `el.value`
    /// JS setter's fast path.
    pub fn set_value(&mut self, node_id: NodeId, value: &str) {
        let mut st = self.state.borrow_mut();
        let existing = st
            .arena
            .get(node_id)
            .and_then(|n| n.attributes.get("value"))
            .cloned()
            .unwrap_or_default();
        if existing == value {
            return;
        }
        if let Some(node) = st.arena.get_mut(node_id) {
            node.attributes.insert("value".to_string(), value.to_string());
        }
        st.dirty.nodes.push(node_id);
    }

    /// Commit a new `value` attribute and mark the node dirty.
    ///
    /// A convenience alias of [`set_value`](Self::set_value) for callers that
    /// always change the value (e.g. a `<select>` selection swapping the shown
    /// label), so the next frame re-cascades and re-renders.
    pub fn set_value_dirty(&mut self, node_id: NodeId, value: &str) {
        self.set_value(node_id, value);
    }

    /// Set the `checked` state of a checkbox / radio and mark changed nodes
    /// dirty. Returns `true` when any `checked` attribute actually changed
    /// (the control itself or a cleared radio sibling).
    ///
    /// Radio buttons: checking one clears the `checked` attribute of every
    /// other input with the same `name` (document-scoped group, matching the
    /// engine's single-form model). Unchecking a radio is a no-op in
    /// browsers, but the engine allows it for checkbox parity — callers never
    /// send it for a click (clicking a selected radio keeps it selected).
    pub fn set_checked(&mut self, node_id: NodeId, checked: bool) -> bool {
        let mut st = self.state.borrow_mut();
        let mut changed = false;
        let is_radio = st
            .arena
            .get(node_id)
            .is_some_and(|n| n.get_type().is_some_and(|t| t.eq_ignore_ascii_case("radio")));
        let name = st
            .arena
            .get(node_id)
            .and_then(|n| n.attributes.get("name").cloned());
        if checked && is_radio {
            if let Some(name) = name {
                // Clear every same-`name` radio sibling (the clicked member
                // is excluded here and set below).
                let siblings: Vec<NodeId> = st
                    .arena
                    .iter()
                    .filter(|(id, n)| {
                        *id != node_id
                            && n.node_type == NodeType::Input
                            && n.get_type()
                                .is_some_and(|t| t.eq_ignore_ascii_case("radio"))
                            && n.attributes.get("name").is_some_and(|nm| *nm == name)
                    })
                    .map(|(id, _)| id)
                    .collect();
                for id in siblings {
                    if let Some(n) = st.arena.get_mut(id) {
                        if n.attributes.shift_remove("checked").is_some() {
                            changed = true;
                            st.dirty.nodes.push(id);
                        }
                    }
                }
            }
        }
        let was = st
            .arena
            .get(node_id)
            .is_some_and(|n| n.attributes.contains_key("checked"));
        if was == checked {
            return changed;
        }
        if let Some(n) = st.arena.get_mut(node_id) {
            if checked {
                n.attributes.insert("checked".to_string(), String::new());
            } else {
                n.attributes.shift_remove("checked");
            }
        }
        st.dirty.nodes.push(node_id);
        true
    }

    /// Replace a `<textarea>`'s text content with `value` and mark the node
    /// dirty. The textarea's DOM value lives in its child text nodes (unlike
    /// `<input>`'s `value` attribute); committing an unchanged value is a
    /// no-op.
    pub fn set_textarea_value(&mut self, node_id: NodeId, value: &str) {
        let mut st = self.state.borrow_mut();
        let existing = st
            .arena
            .get(node_id)
            .map(|n| n.text_content(&st.arena))
            .unwrap_or_default();
        if existing == value {
            return;
        }
        let old_children: Vec<NodeId> = st
            .arena
            .get_mut(node_id)
            .map(|n| n.children.drain(..).collect())
            .unwrap_or_default();
        for child in old_children {
            if let Some(c) = st.arena.get_mut(child) {
                c.parent = None;
            }
        }
        let text_id = st.arena.insert(Node::text(value));
        Node::append_child(&mut st.arena, node_id, text_id);
        st.dirty.nodes.push(node_id);
    }

    /// Toggle the `open` attribute of a `<details>` element and mark it dirty.
    ///
    /// The native disclosure-widget default action (clicking the `<summary>`)
    /// calls this: the presence of `open` decides whether the element renders
    /// its content, so the dirty marker routes a re-render through the usual
    /// `RenderArena` path.
    pub fn toggle_details(&mut self, node_id: NodeId) {
        let mut st = self.state.borrow_mut();
        let Some(node) = st.arena.get_mut(node_id) else {
            return;
        };
        if node.attributes.contains_key("open") {
            node.attributes.shift_remove("open");
        } else {
            node.attributes.insert("open".to_string(), String::new());
        }
        st.dirty.nodes.push(node_id);
    }

    /// Close a `<dialog>` by removing its `open` attribute and marking it
    /// dirty. Returns whether the dialog was actually open (so callers can
    /// skip the `close` event for already-closed dialogs).
    pub fn close_dialog(&mut self, node_id: NodeId) -> bool {
        let mut st = self.state.borrow_mut();
        let Some(node) = st.arena.get_mut(node_id) else {
            return false;
        };
        if node.attributes.shift_remove("open").is_none() {
            return false;
        }
        st.dirty.nodes.push(node_id);
        true
    }

    /// Dispatch a DOM event along the hit region's ancestor path.
    ///
    /// Builds a JS `Event` object, walks `region.path` target-first and fires
    /// every listener registered (via `addEventListener` or `on*`) for each
    /// element, honouring `stopPropagation()`. Returns any render commands
    /// pushed by host functions during the dispatch.
    pub fn fire(
        &mut self,
        region: &HitRegion,
        event_type: EventType,
        x: u32,
        y: u32,
    ) -> Vec<RenderCmd> {
        self.dispatch(region, event_type, x, y, None).0
    }

    /// Dispatch a keyboard event ([`EventType::KeyDown`] / [`EventType::KeyUp`])
    /// along the hit region's ancestor path.
    ///
    /// Like [`fire`](Self::fire) but attaches the [`KeyEventInfo`] payload so
    /// JS listeners read `event.key` / `event.code` / `event.ctrlKey` / …;
    /// coordinates are meaningless for keys and report `0, 0`.
    pub fn fire_key(
        &mut self,
        region: &HitRegion,
        event_type: EventType,
        key: &KeyEventInfo,
    ) -> Vec<RenderCmd> {
        self.fire_key_gated(region, event_type, key).0
    }

    /// Gated variant of [`fire_key`](Self::fire_key): additionally reports
    /// whether any listener on the bubbling path called `event.preventDefault()`,
    /// so the input loop can skip the built-in shortcut for that key.
    pub(crate) fn fire_key_gated(
        &mut self,
        region: &HitRegion,
        event_type: EventType,
        key: &KeyEventInfo,
    ) -> (Vec<RenderCmd>, bool) {
        self.dispatch(region, event_type, 0, 0, Some(key))
    }

    /// Shared dispatch core of [`fire`](Self::fire) and
    /// [`fire_key`](Self::fire_key): build the JS event object (with keyboard
    /// fields when `key` is set), walk `region.path` target-first firing every
    /// registered listener, honouring `stopPropagation()`. Returns any render
    /// commands pushed by host functions during the dispatch, plus whether a
    /// listener called `event.preventDefault()`.
    fn dispatch(
        &mut self,
        region: &HitRegion,
        event_type: EventType,
        x: u32,
        y: u32,
        key: Option<&KeyEventInfo>,
    ) -> (Vec<RenderCmd>, bool) {
        let target_id = region.path.first().and_then(|e| e.id.clone());

        let event = match self.build_event_object(event_type, &region.path, x, y, key) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Failed to build JS event object: {e}");
                return (Vec::new(), false);
            }
        };

        {
            let mut st = self.state.borrow_mut();
            st.current_event = Some(HtmlEvent {
                event_type,
                target: target_id,
                current_target: None,
                x,
                y,
                bubbles: true,
                key: key.cloned(),
            });
        }

        for element in &region.path {
            // The element wrapper doubles as the listener's `this`: browsers
            // bind `this` inside a listener to the element it is attached to
            // (the current target of the dispatch).
            let Ok(wrapper) = node_wrapper(&mut self.context, element.node_id) else {
                log::warn!("Failed to wrap node {:?} during dispatch", element.node_id);
                continue;
            };
            // `currentTarget` must track the element being dispatched.
            if let Some(obj) = event.as_object()
                && let Err(e) =
                    obj.set(js_string!("currentTarget"), wrapper.clone(), false, &mut self.context)
                {
                    log::warn!("Failed to set event.currentTarget: {e}");
                }
            
            if let Some(ev) = &mut self.state.borrow_mut().current_event {
                ev.current_target = element.id.clone();
            }

            let listeners = match get_listeners(&mut self.context, element.node_id, event_type) {
                Ok(l) => l,
                Err(e) => {
                    log::warn!("Failed to read listeners for {:?}: {e}", element.node_id);
                    Vec::new()
                }
            };

            let mut stopped = false;
            for listener in listeners {
                let call = listener
                    .to_object(&mut self.context)
                    .and_then(|f| f.call(&wrapper, std::slice::from_ref(&event), &mut self.context));
                if let Err(e) = call {
                    log::warn!("Listener threw: {e}");
                }
                if let Some(obj) = event.as_object()
                    && let Ok(v) = obj.get(js_string!("_stopped"), &mut self.context)
                    && v.is_boolean()
                    && v.to_boolean()
                {
                    stopped = true;
                    break;
                }
            }
            if stopped {
                break;
            }
        }

        // Any listener may have called `preventDefault()` — the input loop
        // reads this verdict to decide whether the built-in shortcut runs.
        let prevented = event
            .as_object()
            .and_then(|obj| obj.get(js_string!("_prevented"), &mut self.context).ok())
            .is_some_and(|v| v.is_boolean() && v.to_boolean());

        let mut st = self.state.borrow_mut();
        st.current_event = None;
        let cmds = std::mem::take(&mut st.render_cmds);
        drop(st);
        (self.intercept_render_html(cmds), prevented)
    }

    /// Evaluate a JS snippet in the engine's global scope.
    ///
    /// Used by tests and diagnostic tooling to probe or seed engine state
    /// (reading globals back, running setup snippets). Returns the completion
    /// value of the snippet.
    pub fn eval_source(&mut self, src: &str) -> JsResult<JsValue> {
        self.context.eval(Source::from_bytes(src.as_bytes()))
    }

    /// Invoke a JS global function by name.
    ///
    /// The name may be a `<script>`-defined function or a registered host
    /// function. Returns any render commands pushed during the call.
    pub fn invoke_action(&mut self, name: &str) -> Vec<RenderCmd> {
        let global = self.context.global_object();
        let f = match global.get(js_string!(name), &mut self.context) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("Failed to resolve action `{name}`: {e}");
                return Vec::new();
            }
        };
        if !f.is_callable() {
            log::warn!("Action `{name}` is not a function");
            return Vec::new();
        }
        if let Some(f) = f
            .to_object(&mut self.context)
            .ok()
            && let Err(e) = f.call(&JsValue::undefined(), &[], &mut self.context)
        {
            log::warn!("Action `{name}` threw: {e}");
        }
        let mut st = self.state.borrow_mut();
        let cmds = std::mem::take(&mut st.render_cmds);
        drop(st);
        self.intercept_render_html(cmds)
    }

    /// Build a JS `Event` object for a dispatch.
    fn build_event_object(
        &mut self,
        event_type: EventType,
        path: &[crate::events::ElementRef],
        x: u32,
        y: u32,
        key: Option<&KeyEventInfo>,
    ) -> JsResult<JsValue> {
        let target = match path.first() {
            Some(el) => node_wrapper(&mut self.context, el.node_id)?,
            None => JsValue::null(),
        };
        let type_str = match event_type {
            EventType::Click => "click",
            EventType::MouseOver => "mouseover",
            EventType::MouseOut => "mouseout",
            EventType::Focus => "focus",
            EventType::Blur => "blur",
            EventType::Input => "input",
            EventType::Change => "change",
            EventType::KeyDown => "keydown",
            EventType::KeyUp => "keyup",
            EventType::Submit => "submit",
            EventType::Reset => "reset",
            EventType::Toggle => "toggle",
            EventType::Close => "close",
        };

        let mut init = ObjectInitializer::new(&mut self.context);
        init.property(js_string!("type"), js_string!(type_str), Attribute::all())
            .property(js_string!("target"), target, Attribute::all())
            .property(js_string!("currentTarget"), JsValue::null(), Attribute::all())
            .property(js_string!("x"), x as f64, Attribute::all())
            .property(js_string!("y"), y as f64, Attribute::all())
            .property(js_string!("bubbles"), true, Attribute::all())
            .property(
                js_string!("_stopped"),
                false,
                Attribute::WRITABLE | Attribute::NON_ENUMERABLE,
            )
            .property(js_string!("defaultPrevented"), false, Attribute::all())
            .property(
                js_string!("_prevented"),
                false,
                Attribute::WRITABLE | Attribute::NON_ENUMERABLE,
            );
        if let Some(k) = key {
            init.property(js_string!("key"), js_string!(k.key.as_str()), Attribute::all())
                .property(js_string!("code"), js_string!(k.code.as_str()), Attribute::all())
                .property(js_string!("keyCode"), k.key_code as f64, Attribute::all())
                .property(js_string!("shiftKey"), k.shift, Attribute::all())
                .property(js_string!("ctrlKey"), k.ctrl, Attribute::all())
                .property(js_string!("altKey"), k.alt, Attribute::all())
                .property(js_string!("metaKey"), k.meta, Attribute::all())
                .property(js_string!("repeat"), k.repeat, Attribute::all());
        }
        init.function(
                NativeFunction::from_fn_ptr(stop_propagation),
                js_string!("stopPropagation"),
                0,
            )
            .function(
                NativeFunction::from_fn_ptr(prevent_default),
                js_string!("preventDefault"),
                0,
            );
        Ok(init.build().into())
    }
}

impl Drop for ScriptEngine {
    fn drop(&mut self) {
        ENGINE_STATE.with(|s| *s.borrow_mut() = None);
    }
}

/// `event.stopPropagation()` — marks the event as stopped.
fn stop_propagation(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Ok(obj) = this.to_object(context) {
        if let Err(e) = obj.set(js_string!("_stopped"), true, false, context) {
            log::warn!("Failed to mark event stopped: {e}");
        }
    }
    Ok(JsValue::undefined())
}

/// Fetch a GC-rooted timer callback from `globalThis.__htuiTimers` by id.
fn timer_callback(context: &mut Context, id: u32) -> JsValue {
    let global = context.global_object();
    let Ok(registry) = global.get(js_string!("__htuiTimers"), context) else {
        return JsValue::undefined();
    };
    let Ok(registry) = registry.to_object(context) else {
        return JsValue::undefined();
    };
    registry
        .get(js_string!(id.to_string()), context)
        .unwrap_or(JsValue::undefined())
}

/// `event.preventDefault()` — marks the event so the input loop can skip the
/// built-in shortcut for that key (the verdict is echoed back in the script
/// thread's response; the input loop waits for it before acting).
fn prevent_default(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    if let Ok(obj) = this.to_object(context) {
        for flag in ["_prevented", "defaultPrevented"] {
            if let Err(e) = obj.set(js_string!(flag), true, false, context) {
                log::warn!("Failed to mark event prevented: {e}");
                break;
            }
        }
    }
    Ok(JsValue::undefined())
}

/// Register one host function from `EventBindings.host` as a JS global.
fn register_host_global(context: &mut Context, name: &str) -> JsResult<()> {
    let name_capture = name.to_owned();
    let native = NativeFunction::from_copy_closure_with_captures(
        |_this, args, name: &String, context| dispatch_host(name, args, context),
        name_capture,
    );
    context.register_global_builtin_callable(js_string!(name), 0, native)
}

/// Native dispatcher for host functions: looks the function up in the shared
/// state and calls it with a [`HostCall`].
fn dispatch_host(name: &str, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let ScriptState {
        host_fns,
        current_event,
        render_cmds,
        ..
    } = &mut *st;
    let Some(f) = host_fns.get_mut(name) else {
        log::warn!("Unknown host function `{name}`");
        return Ok(JsValue::undefined());
    };
    let mut call = host::HostCall {
        args,
        event: current_event.as_ref(),
        render: render_cmds,
        context,
    };
    f(&mut call)
}

/// Walk the arena and compile every `on*` attribute body into a listener,
/// exactly like a browser does.
fn compile_attribute_listeners(
    context: &mut Context,
    state: &Rc<RefCell<ScriptState>>,
) -> JsResult<()> {
    let node_ids: Vec<NodeId> = state.borrow().arena.iter().map(|(id, _)| id).collect();
    for node_id in node_ids {
        let attrs = {
            let st = state.borrow();
            st.arena.get(node_id).map(|n| {
                [
                    ("onclick", EventType::Click),
                    ("onmouseover", EventType::MouseOver),
                    ("onmouseout", EventType::MouseOut),
                    ("onfocus", EventType::Focus),
                    ("onblur", EventType::Blur),
                    ("oninput", EventType::Input),
                    ("onchange", EventType::Change),
                    ("onkeydown", EventType::KeyDown),
                    ("onkeyup", EventType::KeyUp),
                    ("onsubmit", EventType::Submit),
                    ("onreset", EventType::Reset),
                    ("ontoggle", EventType::Toggle),
                    ("onclose", EventType::Close),
                ]
                .iter()
                .filter_map(|(attr, et)| n.attributes.get(*attr).cloned().map(|b| (b, *et)))
                .collect::<Vec<_>>()
            })
        };
        let Some(attr_bodies) = attrs else { continue };

        for (body, event_type) in attr_bodies {
            // `(function (event) { <body> })` — attribute value is *code*.
            let wrapped = format!("(function (event) {{ {body} }})");
            let fn_val = match context.eval(Source::from_bytes(wrapped.as_bytes())) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("Failed to compile attribute handler `{body}`: {e}");
                    continue;
                }
            };
            if let Err(e) = add_listener(context, node_id, event_type, &fn_val) {
                log::warn!("Failed to register attribute handler for {:?}: {e}", node_id);
            }
        }
    }
    Ok(())
}
