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

use boa_engine::{Context, JsError, JsResult, JsValue, NativeFunction, Source, js_string};
use boa_engine::object::ObjectInitializer;
use boa_engine::property::Attribute;

use crate::{
    events::{AutoRefresh, EventBindings, EventType, HitRegion, HtmlEvent},
    html::html_parse::Parser,
    tdom::node::{NodeArena, NodeId},
    tui::graph_api::RenderCmd,
};

mod dom;
mod host;
mod msg;

pub mod convert;

pub use convert::{FromJs, IntoJs, JsObjectBuilder};
pub use host::{HostCall, HostFn};
pub use msg::{ScriptDirty, ScriptEvent, ScriptMsg, ScriptResponse, apply_message, run_script_thread};

use dom::{add_listener, element_wrapper, get_listeners, install_builtins, with_state};

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
}

thread_local! {
    /// The active engine state for the current thread (the input loop).
    pub(crate) static ENGINE_STATE: RefCell<Option<Rc<RefCell<ScriptState>>>> = RefCell::new(None);
}

/// Browser-aligned JS engine attached to a live DOM.
pub struct ScriptEngine {
    context: Context,
    state: Rc<RefCell<ScriptState>>,
    auto: Option<AutoRefresh>,
}

/// Bootstrap script: the GC-rooted listener + element registries.
const BOOTSTRAP: &str = r#"
globalThis.__htuiListeners = {};
globalThis.__htuiElements = {};
globalThis.__htuiAddListener = function (nodeId, type, fn) {
  var entry = globalThis.__htuiListeners[nodeId] || (globalThis.__htuiListeners[nodeId] = {});
  var arr = entry[type] || (entry[type] = []);
  arr.push(fn);
};
globalThis.__htuiGetListeners = function (nodeId, type) {
  var entry = globalThis.__htuiListeners[nodeId];
  return entry ? (entry[type] || []) : [];
};
"#;

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
        Ok(ScriptEngine {
            context,
            state,
            auto: bindings.auto,
        })
    }

    /// Replace the live document with a freshly parsed one and re-run its
    /// scripts, keeping the session's host functions and auto-refresh config.
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
        let state = Rc::new(RefCell::new(ScriptState {
            arena: doc.arena,
            root_id: doc.root_id,
            render_cmds: Vec::new(),
            dirty: DirtyInfo::default(),
            current_event: None,
            host_fns,
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

    /// The periodic auto-refresh configuration, if any.
    pub fn auto(&self) -> Option<AutoRefresh> {
        self.auto.clone()
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
        let target_id = region.path.first().and_then(|e| e.id.clone());

        let event = match self.build_event_object(event_type, &region.path, x, y) {
            Ok(e) => e,
            Err(e) => {
                log::warn!("Failed to build JS event object: {e}");
                return Vec::new();
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
            });
        }

        for element in &region.path {
            // `currentTarget` must track the element being dispatched.
            if let Ok(wrapper) = element_wrapper(&mut self.context, element.node_id)
                && let Some(obj) = event.as_object()
            {
                if let Err(e) =
                    obj.set(js_string!("currentTarget"), wrapper, false, &mut self.context)
                {
                    log::warn!("Failed to set event.currentTarget: {e}");
                }
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
                    .and_then(|f| f.call(&JsValue::undefined(), &[event.clone()], &mut self.context));
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

        let mut st = self.state.borrow_mut();
        st.current_event = None;
        let cmds = std::mem::take(&mut st.render_cmds);
        drop(st);
        self.intercept_render_html(cmds)
    }

    /// Evaluate a JS snippet in the engine's global scope.
    ///
    /// Used by tests and diagnostic tooling to probe or seed engine state
    /// (reading globals back, running setup snippets). Returns the completion
    /// value of the snippet.
    pub fn eval_source(&mut self, src: &str) -> JsResult<JsValue> {
        self.context.eval(Source::from_bytes(src.as_bytes()))
    }

    /// Invoke a JS global function by name (backs the periodic auto-refresh).
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
    ) -> JsResult<JsValue> {
        let target = match path.first() {
            Some(el) => element_wrapper(&mut self.context, el.node_id)?,
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
            .function(
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
    if let Some(obj) = this.to_object(context).ok() {
        if let Err(e) = obj.set(js_string!("_stopped"), true, false, context) {
            log::warn!("Failed to mark event stopped: {e}");
        }
    }
    Ok(JsValue::undefined())
}

/// `event.preventDefault()` — accepted for API parity, no default action exists.
fn prevent_default(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> JsResult<JsValue> {
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
