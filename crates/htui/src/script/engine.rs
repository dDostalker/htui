use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use boa_engine::{Context, JsResult, JsValue, Source, js_string};

use crate::{
    events::{EventBindings, EventType, HitRegion, HtmlEvent, KeyEventInfo},
    html::html_parse::{ParsedDocument, Parser},
    tdom::node::{Node, NodeArena, NodeId, NodeType},
    tui::graph_api::{DirtyInfo, RenderCmd},
};

use super::bindings::{compile_attribute_listeners, register_host_global};
use super::dom::{get_attr_handler, get_listeners, install_builtins, node_wrapper};
use super::host::HostFn;
use super::state::{ENGINE_STATE, ScriptState};
use super::{EngineBuildError, EngineParts};

/// Browser-aligned JS engine attached to a live DOM.
pub struct ScriptEngine {
    pub(super) context: Context,
    pub(super) state: Rc<RefCell<ScriptState>>,
}

/// Bootstrap script: the GC-rooted listener / element / timer registries.
const BOOTSTRAP: &str = include_str!("bootstrap.js");

impl ScriptEngine {
    /// Create an engine from an HTML document and the application bindings.
    ///
    /// Parses the document , installs the DOM globals and host functions,
    /// compiles on* attribute bodies into listeners, then runs every <script> block.
    /// Errors are returned as Boa errors ,
    /// individual script failures log a warning and continue.
    pub fn new(html: &str, bindings: EventBindings) -> JsResult<Self> {
        let document = {
            let mut parser = Parser::new(html);
            parser.parse_with_scripts()
        };
        let (context, state) = match Self::build_engine(document, bindings.host) {
            Ok(v) => v,
            Err((e, _)) => return Err(e),
        };
        Ok(ScriptEngine { context, state })
    }

    /// Create an engine from an already-parsed document.
    ///
    /// This is the startup path used when the caller wants to parse the HTML
    /// once and distribute independent document snapshots to the script and
    /// render threads.
    pub fn new_from_document(document: ParsedDocument, bindings: EventBindings) -> JsResult<Self> {
        let (context, state) = match Self::build_engine(document, bindings.host) {
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
        let document = {
            let mut parser = Parser::new(html);
            parser.parse_with_scripts()
        };
        let (context, state) = match Self::build_engine(document, host_fns) {
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
        document: ParsedDocument,
        host_fns: HashMap<String, HostFn>,
    ) -> Result<EngineParts, EngineBuildError> {
        let doc = document;
        let (fetch_tx, fetch_rx) = ScriptState::fetch_channel();
        let state = Rc::new(RefCell::new(ScriptState {
            arena: (*doc.arena).clone(),
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

        // run boa，add limits
        let mut context = Context::default();
        let limits = context.runtime_limits_mut();
        limits.set_recursion_limit(512);
        limits.set_loop_iteration_limit(1_000_000);

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
        for script in doc.scripts.iter() {
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
    pub(super) fn intercept_render_html(&mut self, cmds: Vec<RenderCmd>) -> Vec<RenderCmd> {
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
            node.attributes
                .insert("value".to_string(), value.to_string());
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
        let is_radio = st.arena.get(node_id).is_some_and(|n| {
            n.get_type()
                .is_some_and(|t| t.eq_ignore_ascii_case("radio"))
        });
        let name = st
            .arena
            .get(node_id)
            .and_then(|n| n.attributes.get("name").cloned());
        if checked
            && is_radio
            && let Some(name) = name
        {
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
                if let Some(n) = st.arena.get_mut(id)
                    && n.attributes.shift_remove("checked").is_some()
                {
                    changed = true;
                    st.dirty.nodes.push(id);
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
            .map(|n| std::mem::take(&mut n.children))
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

        for element in region.path.iter() {
            // The element wrapper doubles as the listener's `this`: browsers
            // bind `this` inside a listener to the element it is attached to
            // (the current target of the dispatch).
            let Ok(wrapper) = node_wrapper(&mut self.context, element.node_id) else {
                log::warn!("Failed to wrap node {:?} during dispatch", element.node_id);
                continue;
            };
            // `currentTarget` must track the element being dispatched.
            if let Some(obj) = event.as_object()
                && let Err(e) = obj.set(
                    js_string!("currentTarget"),
                    wrapper.clone(),
                    false,
                    &mut self.context,
                )
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
            // `stopImmediatePropagation` only — `stopPropagation` lets the
            // remaining listeners of the *current* element run and just ends
            // the bubbling afterwards (browser semantics).
            let mut immediate = false;
            for listener in listeners {
                let call = listener.to_object(&mut self.context).and_then(|f| {
                    f.call(&wrapper, std::slice::from_ref(&event), &mut self.context)
                });
                if let Err(e) = call {
                    log::warn!("Listener threw: {e}");
                }
                if self.event_flag(&event, "_stoppedImmediate") {
                    immediate = true;
                    break;
                }
            }

            // The `on*` attribute handler is a single slot per (element,
            // event type); per the DOM spec it runs after that element's
            // `addEventListener` listeners, still inside this element's leg
            // of the bubbling path — suppressed only by an immediate stop.
            if !immediate {
                let handler = match get_attr_handler(&mut self.context, element.node_id, event_type)
                {
                    Ok(h) => h,
                    Err(e) => {
                        log::warn!(
                            "Failed to read attribute handler for {:?}: {e}",
                            element.node_id
                        );
                        None
                    }
                };
                if let Some(handler) = handler {
                    let call = handler.to_object(&mut self.context).and_then(|f| {
                        f.call(&wrapper, std::slice::from_ref(&event), &mut self.context)
                    });
                    if let Err(e) = call {
                        log::warn!("Attribute handler threw: {e}");
                    }
                    if self.event_flag(&event, "_stoppedImmediate") {
                        immediate = true;
                    }
                }
            }

            if immediate || self.event_flag(&event, "_stopped") {
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

    /// Read a boolean control flag (`_stopped` / `_stoppedImmediate` /
    /// `_prevented`) off the in-flight event object.
    fn event_flag(&mut self, event: &JsValue, name: &str) -> bool {
        event
            .as_object()
            .and_then(|obj| obj.get(js_string!(name), &mut self.context).ok())
            .is_some_and(|v| v.is_boolean() && v.to_boolean())
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
        if let Some(f) = f.to_object(&mut self.context).ok()
            && let Err(e) = f.call(&JsValue::undefined(), &[], &mut self.context)
        {
            log::warn!("Action `{name}` threw: {e}");
        }
        let mut st = self.state.borrow_mut();
        let cmds = std::mem::take(&mut st.render_cmds);
        drop(st);
        self.intercept_render_html(cmds)
    }

    /// Build a JS `Event` object for a dispatch: the shared object from
    /// [`super::dom::make_event_object`] plus the keyboard payload fields
    /// when a [`KeyEventInfo`] rides along.
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
        let event = super::dom::make_event_object(&mut self.context, event_type, &target, x, y)?;
        if let Some(k) = key
            && let Some(obj) = event.as_object()
        {
            let fields: [(&str, JsValue); 8] = [
                ("key", js_string!(k.key.as_str()).into()),
                ("code", js_string!(k.code.as_str()).into()),
                ("keyCode", JsValue::from(k.key_code as f64)),
                ("shiftKey", JsValue::from(k.shift)),
                ("ctrlKey", JsValue::from(k.ctrl)),
                ("altKey", JsValue::from(k.alt)),
                ("metaKey", JsValue::from(k.meta)),
                ("repeat", JsValue::from(k.repeat)),
            ];
            for (name, value) in fields {
                obj.set(js_string!(name), value, false, &mut self.context)?;
            }
        }
        Ok(event)
    }
}

impl Drop for ScriptEngine {
    fn drop(&mut self) {
        ENGINE_STATE.with(|s| *s.borrow_mut() = None);
    }
}
