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
mod view;
mod wrapper;

// Kept reachable for `super::dom::…` imports from engine.rs / bindings.rs /
// msg.rs (crate-internal API surface); not every item is used today.
pub(crate) use builtins::install_builtins;
#[allow(unused_imports)]
pub(crate) use events::{
    add_listener, clear_attr_handler, clear_node_attr_handler, clear_node_listeners,
    get_attr_handler, get_listeners, make_event_object, parse_event_type, remove_listener,
    set_attr_handler,
};
pub(crate) use fetch::response_object;
pub(crate) use view::{get_window_listeners, mql_last_matches, mql_matches_now, mql_query};

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
    u64_to_node_id(v as u64)
}

/// Recover a `NodeId` from its opaque slotmap bits (the shared UI state
/// carries the focused field's id this way).
pub(crate) fn u64_to_node_id(v: u64) -> NodeId {
    slotmap::KeyData::from_ffi(v).into()
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

/// Create (or return the live cached) JS wrapper for any DOM node (`node_id`).
///
/// Wrappers are cached as `WeakRef`s in `globalThis.__htuiElements[nodeId]`:
/// identity (`el === event.target`) holds while the page holds the wrapper,
/// and once the page drops it the cache entry — with the wrapper — becomes
/// collectible. This keeps the `innerHTML` sweep's reachability oracle
/// ([`wrapper_is_cached`]) accurate instead of pinning every touched node
/// (and its whole subtree) in the arena forever.
pub(crate) fn node_wrapper(context: &mut Context, node_id: NodeId) -> JsResult<JsValue> {
    let key = node_id_to_f64(node_id);
    let registry = registry_elements(context)?;
    let cached = registry.get(key, context)?;
    if let Some(live) = weak_cached_wrapper(context, &cached)? {
        return Ok(live);
    }
    let wrapper = make_node_wrapper(context, node_id)?;
    cache_weak_wrapper(context, &registry, key, &wrapper)?;
    Ok(wrapper)
}

/// The live wrapper behind a wrapper-cache entry, or `None` when the entry is
/// absent or its target has been collected.
fn weak_cached_wrapper(context: &mut Context, entry: &JsValue) -> JsResult<Option<JsValue>> {
    if entry.is_undefined() {
        return Ok(None);
    }
    let Some(entry_obj) = entry.as_object() else {
        return Ok(None);
    };
    let deref = entry_obj.get(js_string!("deref"), context)?;
    let Some(deref_fn) = deref.as_callable() else {
        return Ok(None);
    };
    let target = deref_fn.call(entry, &[], context)?;
    if target.is_undefined() {
        return Ok(None);
    }
    Ok(Some(target))
}

/// Store `wrapper` under `key` wrapped in a JS `WeakRef`.
fn cache_weak_wrapper(
    context: &mut Context,
    registry: &JsObject,
    key: f64,
    wrapper: &JsValue,
) -> JsResult<()> {
    let ctor = context.intrinsics().constructors().weak_ref().constructor();
    let weak = ctor.construct(&[wrapper.clone()], None, context)?;
    registry.set(key, weak, false, context)?;
    // The `WeakRef` constructor registers its target in the engine's
    // kept-alive list (the spec's same-tick resurrection guard). Nothing in
    // this engine relies on that edge, and leaving the wrapper pinned would
    // defeat the weak cache, so flush the list right away.
    context.clear_kept_objects();
    Ok(())
}

/// Drop every wrapper-cache entry whose `WeakRef` target has been collected.
///
/// Called by the `innerHTML` sweep so the registry cannot grow without bound
/// across repeated rewrites that wrap fresh nodes.
pub(super) fn purge_dead_wrappers(context: &mut Context) -> JsResult<()> {
    let registry = registry_elements(context)?;
    for key in registry.own_property_keys(context)? {
        let entry = registry.get(key.clone(), context)?;
        if !entry.is_undefined() && weak_cached_wrapper(context, &entry)?.is_none() {
            registry.delete_property_or_throw(key, context)?;
        }
    }
    Ok(())
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

/// Whether a live JS wrapper still exists for `node_id`.
///
/// The cache stores `WeakRef`s (see [`node_wrapper`]), so this is the
/// accurate "JS still holds this node" oracle used by the `innerHTML` sweep
/// (see `content::sweep_replaced_subtrees`): a dead entry is deleted on
/// check.
pub(super) fn wrapper_is_cached(context: &mut Context, node_id: NodeId) -> JsResult<bool> {
    let key = node_id_to_f64(node_id);
    let registry = registry_elements(context)?;
    let cached = registry.get(key, context)?;
    if weak_cached_wrapper(context, &cached)?.is_some() {
        return Ok(true);
    }
    if !cached.is_undefined() {
        registry.delete_property_or_throw(key, context)?;
    }
    Ok(false)
}

// ============================================================
// Helpers
// ============================================================

/// Concatenate all descendant `Text` nodes of `node_id` (browser
/// `textContent`). A text node's own data is included — the browser returns
/// it for `textContent` on the node itself — so this wraps
/// [`Node::text_content`](crate::tdom::node::Node::text_content) (descendants
/// only) with the self case.
fn collect_text(arena: &NodeArena, node_id: NodeId) -> String {
    let Some(node) = arena.get(node_id) else {
        return String::new();
    };
    if node.node_type == NodeType::Text {
        return node.data.clone().unwrap_or_default();
    }
    node.text_content(arena)
}
