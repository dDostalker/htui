//! Browser-aligned DOM bindings over the Boa engine.
//!
//! Exposes a small DOM surface to scripts:
//! - the global `document` (`getElementById`, `createElement`, `body`);
//! - element wrappers with `textContent`, `style`, `className`, `id`,
//!   `parentNode`, `getAttribute`/`setAttribute`/`removeAttribute`,
//!   `addEventListener`/`removeEventListener`, `appendChild`/`removeChild`;
//! - the `console` object (logs through `log`);
//! - `alert` / `quit` built-ins.
//!
//! Every native here is a `Copy` closure or plain fn pointer (Boa forbids
//! non-`Copy` captures without `unsafe`); shared engine state is fetched from
//! the `thread_local!` registry installed by [`super::ScriptEngine`].

mod attrs;
mod builtins;
mod console;
mod content;
mod document;
mod events;
mod fetch;
mod global_attrs;
mod mutation;
mod style;
mod timers;
mod traverse;
mod wrapper;

// Kept reachable for `super::dom::…` imports from engine.rs / bindings.rs /
// msg.rs (crate-internal API surface); not every item is used today.
pub(crate) use builtins::install_builtins;
#[allow(unused_imports)]
pub(crate) use events::{add_listener, get_listeners, parse_event_type, remove_listener};
pub(crate) use fetch::response_object;

use boa_engine::{
    Context, JsNativeError, JsObject, JsResult, JsValue, NativeFunction, js_string,
    object::{FunctionObjectBuilder, ObjectInitializer, builtins::JsFunction},
};
use slotmap::Key;

use crate::tdom::node::{NodeArena, NodeId, NodeType};

use super::ScriptState;

use wrapper::make_node_wrapper;

/// Fetch the shared engine state (arena, render queue, …).
///
/// Natives call this at the top of every entry point. Returns a clone of the
/// shared `Rc` so the borrow of the `thread_local` is short-lived.
pub(crate) fn with_state() -> Option<std::rc::Rc<std::cell::RefCell<ScriptState>>> {
    super::ENGINE_STATE.with(|s| s.borrow().clone())
}

/// Record a DOM mutation: remember the node so the render thread can repaint
/// the smallest region that covers its affected boxes.
pub(crate) fn mark_dirty(st: &mut ScriptState, node_id: NodeId) {
    st.dirty.nodes.push(node_id);
}

/// Convert a `NodeId` into a JS-safe `f64` (opaque, round-trippable).
pub(crate) fn node_id_to_f64(id: NodeId) -> f64 {
    Key::data(&id).as_ffi() as f64
}

/// Recover a `NodeId` from the `f64` stored on an element wrapper.
pub(crate) fn f64_to_node_id(v: f64) -> NodeId {
    slotmap::KeyData::from_ffi(v as u64).into()
}

/// Read `_nodeId` off a `this` object.
fn this_node_id(this: &JsValue, context: &mut Context) -> JsResult<NodeId> {
    let obj = this
        .to_object(context)
        .map_err(|_| JsNativeError::typ().with_message("expected an element object"))?;
    let v = obj.get(js_string!("_nodeId"), context)?;
    let ffi = v
        .to_number(context)
        .map_err(|_| JsNativeError::typ().with_message("element has no _nodeId"))?;
    Ok(f64_to_node_id(ffi))
}

/// Build the JS function object for a native fn pointer.
fn js_fn(context: &mut Context, native: NativeFunction) -> JsFunction {
    FunctionObjectBuilder::new(context.realm(), native).build()
}

/// Create (or return the cached) JS wrapper for any DOM node (`node_id`).
///
/// Wrappers are cached on the JS side (`globalThis.__htuiElements[nodeId]`) so
/// `el === event.target` identity checks work; one builder serves every node
/// kind (elements, text, comments, the document root, fragments).
pub(crate) fn node_wrapper(context: &mut Context, node_id: NodeId) -> JsResult<JsValue> {
    let key = node_id_to_f64(node_id);
    let registry = registry_elements(context)?;
    let cached = registry.get(key, context)?;
    if !cached.is_undefined() {
        return Ok(cached);
    }
    let wrapper = make_node_wrapper(context, node_id)?;
    registry.set(key, wrapper.clone(), false, context)?;
    Ok(wrapper)
}

/// The JS-side element cache object.
fn registry_elements(context: &mut Context) -> JsResult<JsObject> {
    let global = context.global_object();
    let obj = global.get(js_string!("__htuiElements"), context)?;
    match obj.to_object(context) {
        Ok(o) => Ok(o),
        Err(_) => {
            let mut init = ObjectInitializer::new(context);
            let o = init.build();
            global.set(js_string!("__htuiElements"), o.clone(), false, context)?;
            Ok(o)
        }
    }
}

// ============================================================
// Helpers
// ============================================================

/// Concatenate all descendant `Text` nodes of `node_id` (browser `textContent`).
fn collect_text(arena: &NodeArena, node_id: NodeId) -> String {
    let mut out = String::new();
    let mut stack = vec![node_id];
    while let Some(id) = stack.pop() {
        let Some(node) = arena.get(id) else { continue };
        if node.node_type == NodeType::Text
            && let Some(s) = &node.data
        {
            out.push_str(s);
        }
        stack.extend(node.children.iter().rev().copied());
    }
    out
}
