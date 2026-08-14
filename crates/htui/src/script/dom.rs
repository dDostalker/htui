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

use std::time::{Duration, Instant};

use boa_engine::{
    Context, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, js_string,
    object::builtins::JsPromise,
    object::{FunctionObjectBuilder, ObjectInitializer, builtins::JsFunction},
    property::{Attribute, PropertyDescriptorBuilder},
};
use slotmap::Key;

use crate::{
    events::EventType,
    html::css_parse::{
        CssSelector, SUPPORTED_PROPERTIES, parse_declaration_text, selector_chain_matches,
    },
    resources,
    tdom::node::{Node, NodeArena, NodeId, NodeType, document_base_href},
    tui::graph_api::RenderCmd,
};

use super::{ScriptState, TimerEntry};

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

/// Build a fresh wrapper object for `node_id`. The surface depends on the
/// node kind: every node exposes the common Node interface (`nodeType` /
/// `nodeName` / `textContent` / `parentNode` / `childNodes` / `appendChild`
/// / …), elements add attribute accessors, and Text / Comment nodes add
/// `data`.
fn make_node_wrapper(context: &mut Context, node_id: NodeId) -> JsResult<JsValue> {
    let node_key = node_id_to_f64(node_id);
    let hidden = Attribute::READONLY | Attribute::NON_ENUMERABLE;

    let (is_element, is_text_like) = {
        let Some(rc) = with_state() else {
            return Ok(JsValue::null());
        };
        let st = rc.borrow();
        let Some(node) = st.arena.get(node_id) else {
            return Ok(JsValue::null());
        };
        (
            node.node_type_constant() == 1,
            matches!(node.node_type, NodeType::Text | NodeType::Comment),
        )
    };

    // Accessor functions must be built before the `ObjectInitializer` takes
    // its `&mut Context` borrow.
    let get_node_type = js_fn(context, NativeFunction::from_fn_ptr(native_get_node_type));
    let get_node_name = js_fn(context, NativeFunction::from_fn_ptr(native_get_node_name));
    let get_node_value = js_fn(context, NativeFunction::from_fn_ptr(native_get_node_value));
    let set_node_value = js_fn(context, NativeFunction::from_fn_ptr(native_set_node_value));
    let get_text = js_fn(context, NativeFunction::from_fn_ptr(native_get_text));
    let set_text = js_fn(context, NativeFunction::from_fn_ptr(native_set_text));
    let get_parent = js_fn(context, NativeFunction::from_fn_ptr(native_get_parent));
    let get_parent_element = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_parent_element),
    );
    let get_child_nodes = js_fn(context, NativeFunction::from_fn_ptr(native_get_child_nodes));
    let get_first_child = js_fn(context, NativeFunction::from_fn_ptr(native_get_first_child));
    let get_last_child = js_fn(context, NativeFunction::from_fn_ptr(native_get_last_child));
    let get_prev_sibling = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_prev_sibling),
    );
    let get_next_sibling = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_next_sibling),
    );
    let get_owner_doc = js_fn(
        context,
        NativeFunction::from_fn_ptr(native_get_owner_document),
    );
    let get_base_uri = js_fn(context, NativeFunction::from_fn_ptr(native_get_base_uri));
    let get_data = js_fn(context, NativeFunction::from_fn_ptr(native_get_data));
    let set_data = js_fn(context, NativeFunction::from_fn_ptr(native_set_data));
    let get_tag = js_fn(context, NativeFunction::from_fn_ptr(native_get_tag_name));
    let get_class = js_fn(context, NativeFunction::from_fn_ptr(native_get_class));
    let set_class = js_fn(context, NativeFunction::from_fn_ptr(native_set_class));
    let get_class_list = js_fn(context, NativeFunction::from_fn_ptr(native_get_class_list));
    let get_id = js_fn(context, NativeFunction::from_fn_ptr(native_get_id));
    let set_id = js_fn(context, NativeFunction::from_fn_ptr(native_set_id));
    let get_style = js_fn(context, NativeFunction::from_fn_ptr(native_get_style));
    let get_value = js_fn(context, NativeFunction::from_fn_ptr(native_get_value));
    let set_value = js_fn(context, NativeFunction::from_fn_ptr(native_set_value));
    let get_inner_html = js_fn(context, NativeFunction::from_fn_ptr(native_get_inner_html));
    let set_inner_html = js_fn(context, NativeFunction::from_fn_ptr(native_set_inner_html));
    let get_hidden = js_fn(context, NativeFunction::from_fn_ptr(native_get_hidden));
    let set_hidden = js_fn(context, NativeFunction::from_fn_ptr(native_set_hidden));
    let get_dataset = js_fn(context, NativeFunction::from_fn_ptr(native_get_dataset));

    let mut init = ObjectInitializer::new(context);
    init.property(js_string!("_nodeId"), node_key, hidden);

    // Common Node interface.
    init.accessor(
        js_string!("nodeType"),
        Some(get_node_type),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("nodeName"),
        Some(get_node_name),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("textContent"),
        Some(get_text),
        Some(set_text),
        Attribute::all(),
    );
    init.accessor(
        js_string!("nodeValue"),
        Some(get_node_value),
        Some(set_node_value),
        Attribute::all(),
    );
    init.accessor(
        js_string!("parentNode"),
        Some(get_parent),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("parentElement"),
        Some(get_parent_element),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("childNodes"),
        Some(get_child_nodes),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("firstChild"),
        Some(get_first_child),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("lastChild"),
        Some(get_last_child),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("previousSibling"),
        Some(get_prev_sibling),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("nextSibling"),
        Some(get_next_sibling),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("ownerDocument"),
        Some(get_owner_doc),
        None,
        Attribute::all(),
    );
    init.accessor(
        js_string!("baseURI"),
        Some(get_base_uri),
        None,
        Attribute::all(),
    );

    init.function(
        NativeFunction::from_fn_ptr(native_has_child_nodes),
        js_string!("hasChildNodes"),
        0,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_append_child),
        js_string!("appendChild"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_remove_child),
        js_string!("removeChild"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_insert_before),
        js_string!("insertBefore"),
        2,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_replace_child),
        js_string!("replaceChild"),
        2,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_clone_node),
        js_string!("cloneNode"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_contains),
        js_string!("contains"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_is_same_node),
        js_string!("isSameNode"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_is_equal_node),
        js_string!("isEqualNode"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_add_event_listener),
        js_string!("addEventListener"),
        2,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_remove_event_listener),
        js_string!("removeEventListener"),
        2,
    );

    if is_element {
        init.accessor(js_string!("tagName"), Some(get_tag), None, Attribute::all());
        init.accessor(
            js_string!("className"),
            Some(get_class),
            Some(set_class),
            Attribute::all(),
        );
        init.accessor(
            js_string!("classList"),
            Some(get_class_list),
            None,
            Attribute::all(),
        );
        init.accessor(
            js_string!("id"),
            Some(get_id),
            Some(set_id),
            Attribute::all(),
        );
        init.accessor(
            js_string!("value"),
            Some(get_value),
            Some(set_value),
            Attribute::all(),
        );
        init.accessor(js_string!("style"), Some(get_style), None, Attribute::all());
        init.accessor(
            js_string!("innerHTML"),
            Some(get_inner_html),
            Some(set_inner_html),
            Attribute::all(),
        );
        init.accessor(
            js_string!("hidden"),
            Some(get_hidden),
            Some(set_hidden),
            Attribute::all(),
        );
        init.accessor(
            js_string!("dataset"),
            Some(get_dataset),
            None,
            Attribute::all(),
        );
        init.function(
            NativeFunction::from_fn_ptr(native_get_attribute),
            js_string!("getAttribute"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_set_attribute),
            js_string!("setAttribute"),
            2,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_remove_attribute),
            js_string!("removeAttribute"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_query_selector),
            js_string!("querySelector"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_query_selector_all),
            js_string!("querySelectorAll"),
            1,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_focus),
            js_string!("focus"),
            0,
        );
        init.function(
            NativeFunction::from_fn_ptr(native_blur),
            js_string!("blur"),
            0,
        );
    }
    if is_text_like {
        init.accessor(
            js_string!("data"),
            Some(get_data),
            Some(set_data),
            Attribute::all(),
        );
    }
    let obj = init.build();
    Ok(obj.into())
}

// ============================================================
// Element natives
// ============================================================

/// Parse a CSS selector string for `querySelector` / `querySelectorAll`,
/// throwing a JS `SyntaxError` on invalid input (browser behavior).
fn parse_query_selector(args: &[JsValue], context: &mut Context) -> JsResult<CssSelector> {
    let raw = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    raw.parse::<CssSelector>().map_err(|e| {
        JsNativeError::syntax()
            .with_message(format!("invalid selector `{raw}`: {e}"))
            .into()
    })
}

/// Document-order (pre-order DFS) list of every node under `scope` matching
/// `selector`. The scope element itself is never considered — matching
/// `document.querySelector` (the root's children) and `element.querySelector`
/// (the element's subtree) semantics.
fn query_selector_nodes(arena: &NodeArena, scope: NodeId, selector: &CssSelector) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = arena[scope].children.iter().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if selector_chain_matches(arena, id, selector) {
            out.push(id);
        }
        stack.extend(arena[id].children.iter().rev().copied());
    }
    out
}

/// `document.querySelector(sel)` / `element.querySelector(sel)` — the first
/// matching element in document order, or `null`. On an element the search is
/// scoped to its subtree; `:hover` never matches (no pointer context).
fn native_query_selector(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let selector = parse_query_selector(args, context)?;
    let scope = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let first = query_selector_nodes(&st.arena, scope, &selector)
        .into_iter()
        .next();
    drop(st);
    match first {
        Some(node_id) => node_wrapper(context, node_id),
        None => Ok(JsValue::null()),
    }
}

/// `document.querySelectorAll(sel)` / `element.querySelectorAll(sel)` — every
/// matching element in document order, returned as a static array of element
/// wrappers (indexable / `.length` / `.forEach`, like a browser NodeList).
fn native_query_selector_all(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let selector = parse_query_selector(args, context)?;
    let scope = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let st = rc.borrow();
    let ids = query_selector_nodes(&st.arena, scope, &selector);
    drop(st);
    let arr = boa_engine::object::builtins::JsArray::new(context);
    for id in ids {
        arr.push(node_wrapper(context, id)?, context)?;
    }
    Ok(arr.into())
}

fn native_get_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = args
        .first()
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy());
    let Some(name) = name else {
        return Err(JsNativeError::typ()
            .with_message("getAttribute expects a string")
            .into());
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let value = st
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get(&name).cloned());
    Ok(value
        .map(|s| js_string!(s).into())
        .unwrap_or(JsValue::null()))
}

fn native_set_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = args
        .first()
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy());
    let value = args
        .get(1)
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy());
    let (Some(name), Some(value)) = (name, value) else {
        return Err(JsNativeError::typ()
            .with_message("setAttribute expects two strings")
            .into());
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if let Some(node) = st.arena.get_mut(node_id) {
        node.attributes.insert(name, value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

fn native_remove_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = args
        .first()
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy());
    let Some(name) = name else {
        return Err(JsNativeError::typ()
            .with_message("removeAttribute expects a string")
            .into());
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if let Some(node) = st.arena.get_mut(node_id) {
        node.attributes.shift_remove(&name);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

/// `el.value` getter — the DOM `value` attribute (the field's committed
/// content; the active edit buffer while focused is engine-side state).
/// `<output>` and `<textarea>` are special-cased: their `value` IDL reflects
/// the text content (a textarea's DOM value lives in its child text nodes,
/// not a `value` attribute).
fn native_get_value(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let value = st
        .arena
        .get(node_id)
        .map(|n| {
            if matches!(n.node_type, NodeType::Output | NodeType::Textarea) {
                n.text_content(&st.arena)
            } else {
                n.attributes.get("value").cloned().unwrap_or_default()
            }
        })
        .unwrap_or_default();
    Ok(js_string!(value).into())
}

/// `el.value` setter — write the DOM `value` attribute, marking the node dirty
/// so the next frame re-renders it. Equal values short-circuit (no re-render),
/// mirroring the `textContent` fast path. `<output>` and `<textarea>` are
/// special-cased: setting `value` replaces the element's text content (the
/// output / textarea IDL contract).
fn native_set_value(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if st
        .arena
        .get(node_id)
        .is_some_and(|n| matches!(n.node_type, NodeType::Output | NodeType::Textarea))
    {
        return set_text_content(&mut st, node_id, &value);
    }
    let existing = st
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get("value"))
        .cloned()
        .unwrap_or_default();
    if existing == value {
        return Ok(JsValue::undefined());
    }
    if let Some(node) = st.arena.get_mut(node_id) {
        node.attributes.insert("value".to_string(), value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

fn native_get_text(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let text = collect_text(&st.arena, node_id);
    Ok(js_string!(text).into())
}

fn native_set_text(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let text = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    set_text_content(&mut st, node_id, &text)
}

/// Shared `textContent` write: replace the node's text, short-circuiting on
/// equal values and reusing a sole text child to avoid arena growth. Also the
/// backing store of the `<output>` `value` setter.
fn set_text_content(st: &mut ScriptState, node_id: NodeId, text: &str) -> JsResult<JsValue> {
    // Text / Comment nodes: `textContent` writes their `data` directly.
    let is_text_like = matches!(
        st.arena.get(node_id).map(|n| n.node_type),
        Some(NodeType::Text | NodeType::Comment)
    );
    if is_text_like {
        let existing = st
            .arena
            .get(node_id)
            .and_then(|n| n.data.clone())
            .unwrap_or_default();
        if existing == text {
            return Ok(JsValue::undefined());
        }
        if let Some(n) = st.arena.get_mut(node_id) {
            n.data = Some(text.to_string());
        }
        mark_dirty(st, node_id);
        return Ok(JsValue::undefined());
    }

    // Fast path: the node already holds exactly one text child — reuse it so
    // repeated `textContent` updates don't grow the arena (each grown node
    // makes the per-frame `RenderArena` clone progressively slower). When the
    // new text equals the existing one, skip entirely so no re-render is
    // scheduled (monitors that refresh identical values avoid full relayouts).
    let reuse_child = st
        .arena
        .get(node_id)
        .filter(|n| n.children.len() == 1)
        .and_then(|n| n.children.first().copied())
        .filter(|&c| matches!(st.arena.get(c).map(|n| n.node_type), Some(NodeType::Text)));
    if let Some(child_id) = reuse_child {
        let existing = st
            .arena
            .get(child_id)
            .and_then(|c| c.data.clone())
            .unwrap_or_default();
        if existing == text {
            return Ok(JsValue::undefined());
        }
        if let Some(c) = st.arena.get_mut(child_id) {
            c.data = Some(text.to_string());
        }
        mark_dirty(st, node_id);
        return Ok(JsValue::undefined());
    }

    // Structural change: detach all children and insert a fresh text node.
    let Some(node) = st.arena.get_mut(node_id) else {
        return Ok(JsValue::undefined());
    };
    let old_children = std::mem::take(&mut node.children);
    for child in old_children {
        if let Some(c) = st.arena.get_mut(child) {
            c.parent = None;
        }
    }
    let text_id = st.arena.insert(Node::text(text));
    if let Some(parent) = st.arena.get_mut(node_id) {
        parent.children.push(text_id);
    }
    if let Some(t) = st.arena.get_mut(text_id) {
        t.parent = Some(node_id);
    }
    mark_dirty(st, node_id);
    Ok(JsValue::undefined())
}

fn native_get_inner_html(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let html = crate::html::html_parse::serialize_inner_html(&st.arena, node_id);
    Ok(js_string!(html).into())
}

fn native_set_inner_html(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let html = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    set_inner_html(&mut st, node_id, &html)
}

/// Shared `innerHTML` write: parse `html` as a fragment in the context of
/// `node_id` and replace the node's children. `<style>` CSS is accumulated on
/// the state so the next render applies it; `<script>` blocks are skipped
/// (never executed, matching the browser's `innerHTML` semantics).
fn set_inner_html(st: &mut ScriptState, node_id: NodeId, html: &str) -> JsResult<JsValue> {
    // `innerHTML` is an Element-only property; non-elements are left alone.
    let is_element = st
        .arena
        .get(node_id)
        .is_some_and(|n| n.node_type_constant() == 1);
    if !is_element {
        return Ok(JsValue::undefined());
    }
    let mut css_sources = Vec::new();
    crate::html::html_parse::parse_fragment_into(&mut st.arena, node_id, html, &mut css_sources);
    if !css_sources.is_empty() {
        st.dirty.style_css.push_str(&css_sources.join("\n"));
    }
    mark_dirty(st, node_id);
    Ok(JsValue::undefined())
}

/// `el.focus()` — request focus for a text field.
///
/// htui's input model focuses `<input>` / `<textarea>` only, so calling
/// `focus()` on any other element is a no-op (browsers ignore it for
/// non-focusable elements). The actual focus transition runs on the input
/// loop — it owns the edit session — so the request travels as a
/// [`RenderCmd::FocusRequest`]; the input loop routes the `focus` event back
/// through the script-message path.
fn native_focus(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let focusable = st
        .arena
        .get(node_id)
        .is_some_and(|n| matches!(n.node_type, NodeType::Input | NodeType::Textarea));
    if !focusable {
        return Ok(JsValue::undefined());
    }
    st.render_cmds.push(RenderCmd::FocusRequest { node_id });
    Ok(JsValue::undefined())
}

/// `el.blur()` — remove focus from this element if it is the focused one
/// (a no-op otherwise, matching the browser). The transition runs on the
/// input loop via [`RenderCmd::BlurRequest`].
fn native_blur(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    st.render_cmds.push(RenderCmd::BlurRequest { node_id });
    Ok(JsValue::undefined())
}

fn native_get_class(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let value = st
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get("class").cloned())
        .unwrap_or_default();
    Ok(js_string!(value).into())
}

fn native_set_class(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if let Some(node) = st.arena.get_mut(node_id) {
        node.attributes.insert("class".to_string(), value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

/// Return a live `DOMTokenList`-like object over the element's `class`
/// attribute (the `classList` property). The factory's `getAttr` / `setAttr`
/// closures read / write the attribute through the engine arena, so the
/// returned object reflects later DOM mutations (browser `classList` is
/// live).
fn native_get_class_list(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let node_key = node_id_to_f64(node_id);
    let get_attr = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(class_list_get, node_key),
    );
    let set_attr = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(class_list_set, node_key),
    );
    let global = context.global_object();
    let helper = global
        .get(js_string!("__htuiClassList"), context)?
        .to_object(context)?;
    helper.call(
        &JsValue::undefined(),
        &[get_attr.into(), set_attr.into()],
        context,
    )
}

/// Live `classList` read: the element's `class` attribute value.
fn class_list_get(
    _this: &JsValue,
    _args: &[JsValue],
    node_key: &f64,
    _context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = f64_to_node_id(*node_key);
    let value = with_state()
        .map(|rc| {
            rc.borrow()
                .arena
                .get(node_id)
                .and_then(|n| n.attributes.get("class").cloned())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    Ok(js_string!(value).into())
}

/// Live `classList` write: replace the element's `class` attribute, marking
/// the node dirty only when the value actually changed (an unchanged token
/// set must not schedule a re-render).
fn class_list_set(
    _this: &JsValue,
    args: &[JsValue],
    node_key: &f64,
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = f64_to_node_id(*node_key);
    let value = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let existing = st
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get("class").cloned())
        .unwrap_or_default();
    if existing == value {
        return Ok(JsValue::undefined());
    }
    if let Some(node) = st.arena.get_mut(node_id) {
        node.attributes.insert("class".to_string(), value);
    }
    mark_dirty(&mut st, node_id);
    Ok(JsValue::undefined())
}

/// `el.hidden` read — whether the `hidden` global attribute is present (the
/// UA `[hidden] { display: none }` rule hides the element).
fn native_get_hidden(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let present = with_state()
        .map(|rc| {
            rc.borrow()
                .arena
                .get(node_id)
                .is_some_and(|n| n.attributes.contains_key("hidden"))
        })
        .unwrap_or(false);
    Ok(JsValue::from(present))
}

/// `el.hidden` write — set / remove the `hidden` attribute, scheduling a
/// re-render only when the state actually changed (browser `hidden` is a
/// boolean attribute: any non-false value is "present").
fn native_set_hidden(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let on = args.first().is_some_and(|v| v.to_boolean());
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let Some(node) = st.arena.get_mut(node_id) else {
        return Ok(JsValue::undefined());
    };
    let present = node.attributes.contains_key("hidden");
    if on && !present {
        node.attributes.insert("hidden".to_string(), String::new());
        mark_dirty(&mut st, node_id);
    } else if !on && present {
        node.attributes.shift_remove("hidden");
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

/// `el.dataset` — a live `DOMStringMap` over the element's `data-*`
/// attributes (the `dataset` property). The factory's native closures read /
/// write individual `data-<name>` attributes through the engine arena, so the
/// returned object reflects later DOM mutations (browser `dataset` is live).
fn native_get_dataset(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let node_key = node_id_to_f64(node_id);
    let get = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(dataset_get, node_key),
    );
    let set = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(dataset_set, node_key),
    );
    let remove = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(dataset_remove, node_key),
    );
    let keys = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(dataset_keys, node_key),
    );
    let global = context.global_object();
    let helper = global
        .get(js_string!("__htuiDataset"), context)?
        .to_object(context)?;
    helper.call(
        &JsValue::undefined(),
        &[get.into(), set.into(), remove.into(), keys.into()],
        context,
    )
}

/// Live `dataset` read: the `data-<name>` attribute value (the `name` is the
/// kebab-cased key passed by the proxy), or `undefined` when absent.
fn dataset_get(
    _this: &JsValue,
    args: &[JsValue],
    node_key: &f64,
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = f64_to_node_id(*node_key);
    let name = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let value = with_state().map(|rc| {
        rc.borrow()
            .arena
            .get(node_id)
            .and_then(|n| n.attributes.get(&format!("data-{name}")).cloned())
    });
    Ok(match value {
        Some(Some(v)) => js_string!(v).into(),
        _ => JsValue::undefined(),
    })
}

/// Live `dataset` write: set `data-<name>` to the value. An empty string
/// removes the attribute (browser `dataset` semantics); the node is marked
/// dirty only when the attribute actually changed.
fn dataset_set(
    _this: &JsValue,
    args: &[JsValue],
    node_key: &f64,
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = f64_to_node_id(*node_key);
    let name = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let value = args
        .get(1)
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let Some(node) = st.arena.get_mut(node_id) else {
        return Ok(JsValue::undefined());
    };
    let key = format!("data-{name}");
    let existing = node.attributes.get(&key).cloned();
    if value.is_empty() {
        if existing.is_some() {
            node.attributes.shift_remove(&key);
            mark_dirty(&mut st, node_id);
        }
    } else if existing.as_deref() != Some(value.as_str()) {
        node.attributes.insert(key, value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

/// Live `dataset` delete: remove the `data-<name>` attribute.
fn dataset_remove(
    _this: &JsValue,
    args: &[JsValue],
    node_key: &f64,
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = f64_to_node_id(*node_key);
    let name = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if let Some(node) = st.arena.get_mut(node_id) {
        let key = format!("data-{name}");
        if node.attributes.shift_remove(&key).is_some() {
            mark_dirty(&mut st, node_id);
        }
    }
    Ok(JsValue::undefined())
}

/// Live `dataset` enumeration: the kebab-cased names of every `data-*`
/// attribute on the element, in attribute order.
fn dataset_keys(
    _this: &JsValue,
    _args: &[JsValue],
    node_key: &f64,
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = f64_to_node_id(*node_key);
    let names: Vec<String> = with_state()
        .map(|rc| {
            rc.borrow()
                .arena
                .get(node_id)
                .map(|n| {
                    n.attributes
                        .keys()
                        .filter_map(|k| k.strip_prefix("data-").map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let arr = boa_engine::object::builtins::JsArray::new(context);
    for name in names {
        arr.push(js_string!(name), context)?;
    }
    Ok(arr.into())
}

fn native_get_id(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let value = st
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get("id").cloned())
        .unwrap_or_default();
    Ok(js_string!(value).into())
}

fn native_set_id(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if let Some(node) = st.arena.get_mut(node_id) {
        node.attributes.insert("id".to_string(), value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

fn native_get_parent(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let parent = st.arena.get(node_id).and_then(|n| n.parent);
    match parent {
        Some(pid) => node_wrapper(context, pid),
        None => Ok(JsValue::null()),
    }
}

// ============================================================
// Node interface natives (shared by every node kind)
// ============================================================

fn native_get_node_type(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let Some(node) = st.arena.get(node_id) else {
        return Ok(JsValue::null());
    };
    Ok(JsValue::from(node.node_type_constant() as u32))
}

fn native_get_node_name(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let Some(node) = st.arena.get(node_id) else {
        return Ok(js_string!().into());
    };
    Ok(js_string!(node.node_name()).into())
}

fn native_get_tag_name(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let name = st
        .arena
        .get(node_id)
        .and_then(|n| n.node_type.tag_name())
        .map(str::to_uppercase)
        .unwrap_or_default();
    Ok(js_string!(name).into())
}

fn native_get_node_value(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let Some(node) = st.arena.get(node_id) else {
        return Ok(JsValue::null());
    };
    match node.node_type {
        NodeType::Text | NodeType::Comment => {
            Ok(js_string!(node.data.as_deref().unwrap_or("")).into())
        }
        _ => Ok(JsValue::null()),
    }
}

fn native_set_node_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let Some(node) = st.arena.get_mut(node_id) else {
        return Ok(JsValue::undefined());
    };
    if matches!(node.node_type, NodeType::Text | NodeType::Comment) {
        node.data = Some(value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

fn native_get_parent_element(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let parent = st.arena.get(node_id).and_then(|n| n.parent).filter(|&pid| {
        st.arena
            .get(pid)
            .map(|p| p.node_type_constant() == 1)
            .unwrap_or(false)
    });
    match parent {
        Some(pid) => node_wrapper(context, pid),
        None => Ok(JsValue::null()),
    }
}

fn native_get_child_nodes(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    call_live_list(context, node_id)
}

/// Build a live NodeList proxy over `node_id`'s children. True liveness:
/// the returned object's `length` / indexed access / enumeration read the
/// arena on every access, so a held reference reflects later DOM mutations.
fn call_live_list(context: &mut Context, node_id: NodeId) -> JsResult<JsValue> {
    let node_key = node_id_to_f64(node_id);
    let get_len = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(nodelist_len, node_key),
    );
    let get_item = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(nodelist_item, node_key),
    );
    let global = context.global_object();
    let helper = global
        .get(js_string!("__htuiLiveList"), context)?
        .to_object(context)?;
    helper.call(
        &JsValue::undefined(),
        &[get_len.into(), get_item.into()],
        context,
    )
}

/// Live `NodeList.length` (reads the arena on every call).
fn nodelist_len(
    _this: &JsValue,
    _args: &[JsValue],
    node_key: &f64,
    _context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = f64_to_node_id(*node_key);
    let len = with_state()
        .map(|rc| {
            rc.borrow()
                .arena
                .get(node_id)
                .map(|n| n.children.len())
                .unwrap_or(0)
        })
        .unwrap_or(0);
    Ok(JsValue::from(len as u32))
}

/// Live `NodeList[i]` (reads the arena on every call).
fn nodelist_item(
    _this: &JsValue,
    args: &[JsValue],
    node_key: &f64,
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = f64_to_node_id(*node_key);
    let idx = args
        .first()
        .and_then(|v| v.to_number(context).ok())
        .unwrap_or(-1.0);
    if idx < 0.0 {
        return Ok(JsValue::null());
    }
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let child = st
        .arena
        .get(node_id)
        .and_then(|n| n.children.get(idx as usize))
        .copied();
    match child {
        Some(cid) => node_wrapper(context, cid),
        None => Ok(JsValue::null()),
    }
}

fn native_get_first_child(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let child = st
        .arena
        .get(node_id)
        .and_then(|n| n.children.first())
        .copied();
    match child {
        Some(cid) => node_wrapper(context, cid),
        None => Ok(JsValue::null()),
    }
}

fn native_get_last_child(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let child = st
        .arena
        .get(node_id)
        .and_then(|n| n.children.last())
        .copied();
    match child {
        Some(cid) => node_wrapper(context, cid),
        None => Ok(JsValue::null()),
    }
}

fn native_get_prev_sibling(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let prev = st
        .arena
        .get(node_id)
        .and_then(|n| n.parent)
        .and_then(|pid| {
            let parent = &st.arena[pid];
            let mut prev = None;
            for &cid in &parent.children {
                if cid == node_id {
                    break;
                }
                prev = Some(cid);
            }
            prev
        });
    match prev {
        Some(cid) => node_wrapper(context, cid),
        None => Ok(JsValue::null()),
    }
}

fn native_get_next_sibling(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let next = st
        .arena
        .get(node_id)
        .and_then(|n| n.parent)
        .and_then(|pid| {
            let parent = &st.arena[pid];
            let mut found = false;
            let mut next = None;
            for &cid in &parent.children {
                if found {
                    next = Some(cid);
                    break;
                }
                if cid == node_id {
                    found = true;
                }
            }
            next
        });
    match next {
        Some(cid) => node_wrapper(context, cid),
        None => Ok(JsValue::null()),
    }
}

fn native_get_owner_document(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    if node_id == st.root_id {
        return Ok(JsValue::null());
    }
    node_wrapper(context, st.root_id)
}

/// `baseURI` — the document base URL established by the first `<base href>`,
/// or an empty string when the document declares no `<base>`. Mirrors the DOM
/// property that reports the base URL relative references (`<img src>`,
/// `<script src>`, `fetch`, …) resolve against.
fn native_get_base_uri(
    _this: &JsValue,
    _args: &[JsValue],
    _context: &mut Context,
) -> JsResult<JsValue> {
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let uri = document_base_href(&st.arena, st.root_id).unwrap_or_default();
    Ok(js_string!(uri).into())
}

fn native_has_child_nodes(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let has = with_state()
        .map(|rc| {
            rc.borrow()
                .arena
                .get(node_id)
                .map(|n| !n.children.is_empty())
                .unwrap_or(false)
        })
        .unwrap_or(false);
    Ok(JsValue::from(has))
}

fn native_insert_before(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let parent_id = this_node_id(this, context)?;
    let child_id = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context)?;
    // `null` (or a missing) reference child appends at the end.
    let reference_id = this_node_id(args.get(1).unwrap_or(&JsValue::undefined()), context).ok();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let position = reference_id.and_then(|ref_id| {
        st.arena
            .get(parent_id)
            .and_then(|p| p.children.iter().position(|&c| c == ref_id))
    });
    // Detach `child_id` from any current parent.
    let old_parent = st.arena.get(child_id).and_then(|c| c.parent);
    if let Some(op) = old_parent
        && let Some(opn) = st.arena.get_mut(op)
    {
        opn.children.retain(|c| *c != child_id);
    }
    if let Some(child) = st.arena.get_mut(child_id) {
        child.parent = Some(parent_id);
    }
    if let Some(parent) = st.arena.get_mut(parent_id) {
        match position {
            Some(pos) => parent.children.insert(pos, child_id),
            None => parent.children.push(child_id),
        }
        mark_dirty(&mut st, parent_id);
        mark_dirty(&mut st, child_id);
    }
    drop(st);
    node_wrapper(context, child_id)
}

fn native_replace_child(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let parent_id = this_node_id(this, context)?;
    let new_child = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let old_child = this_node_id(args.get(1).unwrap_or(&JsValue::undefined()), context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    // Detach `new_child` from its current parent (if any).
    let old_parent = st.arena.get(new_child).and_then(|c| c.parent);
    if let Some(op) = old_parent
        && let Some(opn) = st.arena.get_mut(op)
    {
        opn.children.retain(|c| *c != new_child);
    }
    if let Some(parent) = st.arena.get_mut(parent_id) {
        match parent.children.iter().position(|&c| c == old_child) {
            Some(pos) => parent.children[pos] = new_child,
            None => parent.children.push(new_child),
        }
    }
    if let Some(c) = st.arena.get_mut(new_child) {
        c.parent = Some(parent_id);
    }
    if let Some(c) = st.arena.get_mut(old_child) {
        c.parent = None;
    }
    mark_dirty(&mut st, parent_id);
    mark_dirty(&mut st, new_child);
    mark_dirty(&mut st, old_child);
    drop(st);
    node_wrapper(context, old_child)
}

fn native_clone_node(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let deep = args.first().map(|v| v.to_boolean()).unwrap_or(false);
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    let Some(clone_id) = clone_subtree(&mut st, node_id, deep) else {
        return Ok(JsValue::null());
    };
    drop(st);
    node_wrapper(context, clone_id)
}

/// Deep (or shallow) copy a node into the arena as a detached subtree.
///
/// `parent` is left `None`, so the clone renders nothing until appended;
/// event listeners are not copied (they are keyed by `NodeId` in the JS
/// registry, and a clone gets fresh ids) — matching browser semantics.
fn clone_subtree(st: &mut ScriptState, node_id: NodeId, deep: bool) -> Option<NodeId> {
    let source = st.arena.get(node_id)?.clone();
    let mut clone = source.clone();
    clone.parent = None;
    clone.children.clear();
    let clone_id = st.arena.insert(clone);
    if deep {
        for &child in &source.children {
            let child_clone = clone_subtree(st, child, true)?;
            st.arena[child_clone].parent = Some(clone_id);
            st.arena[clone_id].children.push(child_clone);
        }
    }
    Some(clone_id)
}

fn native_contains(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(other_id) = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context).ok()
    else {
        return Ok(JsValue::from(false));
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::from(false));
    };
    let st = rc.borrow();
    // A node contains itself and all of its descendants.
    let mut cur = Some(other_id);
    while let Some(id) = cur {
        if id == node_id {
            return Ok(JsValue::from(true));
        }
        cur = st.arena.get(id).and_then(|n| n.parent);
    }
    Ok(JsValue::from(false))
}

fn native_is_same_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let other = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context).ok();
    Ok(JsValue::from(other == Some(node_id)))
}

fn native_is_equal_node(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(other_id) = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context).ok()
    else {
        return Ok(JsValue::from(false));
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::from(false));
    };
    let st = rc.borrow();
    Ok(JsValue::from(nodes_equal(&st.arena, node_id, other_id)))
}

/// Structural equality: same node kind / data / attributes (order-insensitive,
/// matching the DOM's set semantics) and equal children recursively.
fn nodes_equal(arena: &NodeArena, a: NodeId, b: NodeId) -> bool {
    let (na, nb) = match (arena.get(a), arena.get(b)) {
        (Some(x), Some(y)) => (x, y),
        _ => return false,
    };
    if na.node_type != nb.node_type || na.data != nb.data {
        return false;
    }
    if na.attributes.len() != nb.attributes.len() {
        return false;
    }
    for (k, v) in &na.attributes {
        if nb.attributes.get(k) != Some(v) {
            return false;
        }
    }
    na.children.len() == nb.children.len()
        && na
            .children
            .iter()
            .zip(&nb.children)
            .all(|(&ca, &cb)| nodes_equal(arena, ca, cb))
}

fn native_get_data(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let data = st
        .arena
        .get(node_id)
        .and_then(|n| n.data.clone())
        .unwrap_or_default();
    Ok(js_string!(data).into())
}

fn native_set_data(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if let Some(n) = st.arena.get_mut(node_id) {
        if n.data.as_deref() == Some(value.as_str()) {
            return Ok(JsValue::undefined());
        }
        n.data = Some(value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

/// Return an object whose accessors read/write the element's `style`
/// attribute (CSS custom-property semantics, camelCase ↔ kebab-case).
fn native_get_style(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let node_key = node_id_to_f64(node_id);
    let hidden = Attribute::READONLY | Attribute::NON_ENUMERABLE;

    // Build all accessor functions before the `ObjectInitializer` takes its
    // `&mut Context` borrow.
    let mut accessors: Vec<(JsString, Option<JsFunction>, Option<JsFunction>)> = Vec::new();
    for (idx, kebab) in SUPPORTED_PROPERTIES.iter().enumerate() {
        let idx = idx as u32;
        let camel = kebab_to_camel(kebab);
        let getter = NativeFunction::from_copy_closure_with_captures(
            |this, _args, idx: &u32, context| {
                style_get(this, &SUPPORTED_PROPERTIES[*idx as usize], context)
            },
            idx,
        );
        let setter = NativeFunction::from_copy_closure_with_captures(
            |this, args, idx: &u32, context| {
                style_set(
                    this,
                    &SUPPORTED_PROPERTIES[*idx as usize],
                    args.first(),
                    context,
                )
            },
            idx,
        );
        let get = js_fn(context, getter);
        let set = js_fn(context, setter);
        accessors.push((js_string!(camel), Some(get), Some(set)));
    }
    // `cssText` gives access to the raw style attribute.
    let get_css = js_fn(context, NativeFunction::from_fn_ptr(style_get_css_text));
    let set_css = js_fn(context, NativeFunction::from_fn_ptr(style_set_css_text));

    let mut init = ObjectInitializer::new(context);
    init.property(js_string!("_nodeId"), node_key, hidden);
    for (name, get, set) in accessors {
        init.accessor(name, get, set, Attribute::all());
    }
    init.accessor(
        js_string!("cssText"),
        Some(get_css),
        Some(set_css),
        Attribute::all(),
    );
    Ok(init.build().into())
}

/// Convert a kebab-case CSS property name to its camelCase JS `style`
/// property name (`background-color` → `backgroundColor`).
fn kebab_to_camel(kebab: &str) -> String {
    let mut out = String::with_capacity(kebab.len());
    let mut upper_next = false;
    for ch in kebab.chars() {
        if ch == '-' {
            upper_next = true;
        } else if upper_next {
            out.extend(ch.to_uppercase());
            upper_next = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// The element's style attribute as a declaration map.
fn style_map(st: &ScriptState, node_id: NodeId) -> indexmap::IndexMap<String, String> {
    let raw = st
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get("style").cloned())
        .unwrap_or_default();
    parse_declaration_text(&raw)
}

fn style_get(this: &JsValue, kebab: &&'static str, context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let value = style_map(&st, node_id)
        .get(*kebab)
        .cloned()
        .unwrap_or_default();
    Ok(js_string!(value).into())
}

fn style_set(
    this: &JsValue,
    kebab: &&'static str,
    value: Option<&JsValue>,
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = value
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let mut map = style_map(&st, node_id);
    if value.is_empty() {
        map.shift_remove(*kebab);
    } else {
        map.insert((*kebab).to_string(), value);
    }
    let serialized: Vec<String> = map.iter().map(|(k, v)| format!("{k}: {v}")).collect();
    if let Some(node) = st.arena.get_mut(node_id) {
        node.attributes
            .insert("style".to_string(), serialized.join("; "));
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

fn style_get_css_text(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let value = st
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get("style").cloned())
        .unwrap_or_default();
    Ok(js_string!(value).into())
}

fn style_set_css_text(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if let Some(node) = st.arena.get_mut(node_id) {
        node.attributes.insert("style".to_string(), value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}

fn native_add_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let event_type = parse_event_type(args.first(), context)?;
    let Some(fn_val) = args.get(1) else {
        return Err(JsNativeError::typ()
            .with_message("addEventListener expects a function")
            .into());
    };
    if fn_val.as_object().is_none() || !fn_val.is_callable() {
        return Err(JsNativeError::typ()
            .with_message("listener is not a function")
            .into());
    }
    add_listener(context, node_id, event_type, fn_val)?;
    Ok(JsValue::undefined())
}

fn native_remove_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let event_type = parse_event_type(args.first(), context)?;
    remove_listener(context, node_id, event_type, args.get(1))?;
    Ok(JsValue::undefined())
}

fn native_append_child(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let parent_id = this_node_id(this, context)?;
    let child_id = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    // Detach from any existing parent.
    let old_parent = st.arena.get(child_id).and_then(|c| c.parent);
    if let Some(old_parent) = old_parent
        && let Some(op) = st.arena.get_mut(old_parent)
    {
        op.children.retain(|c| *c != child_id);
    }
    if let Some(child) = st.arena.get_mut(child_id) {
        child.parent = Some(parent_id);
    }
    if let Some(parent) = st.arena.get_mut(parent_id) {
        parent.children.push(child_id);
        mark_dirty(&mut st, parent_id);
        mark_dirty(&mut st, child_id);
    }
    Ok(JsValue::undefined())
}

fn native_remove_child(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let parent_id = this_node_id(this, context)?;
    let child_id = this_node_id(args.first().unwrap_or(&JsValue::undefined()), context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if let Some(child) = st.arena.get_mut(child_id) {
        child.parent = None;
    }
    if let Some(parent) = st.arena.get_mut(parent_id) {
        parent.children.retain(|c| *c != child_id);
        mark_dirty(&mut st, parent_id);
        mark_dirty(&mut st, child_id);
    }
    Ok(JsValue::undefined())
}

// ============================================================
// Listener registry (JS-side, GC-safe)
// ============================================================

fn event_type_str(et: EventType) -> &'static str {
    match et {
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
    }
}

pub(crate) fn parse_event_type(
    arg: Option<&JsValue>,
    context: &mut Context,
) -> JsResult<EventType> {
    let Some(arg) = arg else {
        return Err(JsNativeError::typ()
            .with_message("missing event type")
            .into());
    };
    let s = arg.to_string(context)?.to_std_string_lossy();
    match s.as_str() {
        "click" => Ok(EventType::Click),
        "mouseover" => Ok(EventType::MouseOver),
        "mouseout" => Ok(EventType::MouseOut),
        "focus" => Ok(EventType::Focus),
        "blur" => Ok(EventType::Blur),
        "input" => Ok(EventType::Input),
        "change" => Ok(EventType::Change),
        "submit" => Ok(EventType::Submit),
        "reset" => Ok(EventType::Reset),
        "toggle" => Ok(EventType::Toggle),
        "close" => Ok(EventType::Close),
        other => Err(JsNativeError::typ()
            .with_message(format!("unsupported event type `{other}`"))
            .into()),
    }
}

/// Register a JS listener for `(node, event_type)` in the JS-side registry.
pub(crate) fn add_listener(
    context: &mut Context,
    node_id: NodeId,
    event_type: EventType,
    listener: &JsValue,
) -> JsResult<()> {
    call_registry_helper(
        context,
        "__htuiAddListener",
        &[
            js_string!(node_id_to_f64(node_id).to_string()).into(),
            js_string!(event_type_str(event_type)).into(),
            listener.clone(),
        ],
    )
}

/// Remove a JS listener (by identity) from the registry.
pub(crate) fn remove_listener(
    context: &mut Context,
    node_id: NodeId,
    event_type: EventType,
    listener: Option<&JsValue>,
) -> JsResult<()> {
    let key = node_id_to_f64(node_id).to_string();
    let global = context.global_object();
    let registry = global
        .get(js_string!("__htuiListeners"), context)?
        .to_object(context)?;
    let node_entry = registry.get(js_string!(key.clone()), context)?;
    if node_entry.is_undefined() {
        return Ok(());
    }
    let node_obj = node_entry.to_object(context)?;
    let arr = node_obj.get(js_string!(event_type_str(event_type)), context)?;
    if arr.is_undefined() {
        return Ok(());
    }
    let arr_obj = arr.to_object(context)?;
    let len = arr_obj
        .get(js_string!("length"), context)?
        .to_u32(context)?;
    if let Some(listener) = listener {
        // Filter out the matching function (identity comparison).
        let mut new_arr = Vec::new();
        for i in 0..len {
            let item = arr_obj.get(i, context)?;
            if item.strict_equals(listener) {
                continue;
            }
            new_arr.push(item);
        }
        let new_js: JsValue = {
            let mut init = ObjectInitializer::new(context);
            new_arr
                .into_iter()
                .enumerate()
                .fold(&mut init, |acc, (i, v)| {
                    acc.property(
                        i,
                        v,
                        Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE,
                    );
                    acc
                });
            init.build().into()
        };
        node_obj.set(
            js_string!(event_type_str(event_type)),
            new_js,
            false,
            context,
        )?;
    }
    Ok(())
}

/// Fetch the listener array for `(node, event_type)` (may be empty).
pub(crate) fn get_listeners(
    context: &mut Context,
    node_id: NodeId,
    event_type: EventType,
) -> JsResult<Vec<JsValue>> {
    let global = context.global_object();
    let helper = global
        .get(js_string!("__htuiGetListeners"), context)?
        .to_object(context)?;
    let arr = helper.call(
        &JsValue::undefined(),
        &[
            js_string!(node_id_to_f64(node_id).to_string()).into(),
            js_string!(event_type_str(event_type)).into(),
        ],
        context,
    )?;
    let arr_obj = arr.to_object(context)?;
    let len = arr_obj
        .get(js_string!("length"), context)?
        .to_u32(context)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        out.push(arr_obj.get(i, context)?);
    }
    Ok(out)
}

/// Call a registry helper defined in the bootstrap script.
fn call_registry_helper(context: &mut Context, name: &str, args: &[JsValue]) -> JsResult<()> {
    let global = context.global_object();
    let helper = global.get(js_string!(name), context)?.to_object(context)?;
    helper.call(&JsValue::undefined(), args, context)?;
    Ok(())
}

// ============================================================
// document / console / built-ins
// ============================================================

/// Install the `document`, `console`, `alert` and `quit` globals.
pub(crate) fn install_builtins(context: &mut Context) -> JsResult<()> {
    // document: the root Document node wrapper plus document-only members.
    let root_id = {
        let Some(rc) = with_state() else {
            return Err(JsNativeError::typ()
                .with_message("script engine has no state")
                .into());
        };
        rc.borrow().root_id
    };
    let document = node_wrapper(context, root_id)?;
    let doc_obj = document.to_object(context)?;

    for (name, native) in [
        (
            "getElementById",
            NativeFunction::from_fn_ptr(native_get_element_by_id),
        ),
        (
            "querySelector",
            NativeFunction::from_fn_ptr(native_query_selector),
        ),
        (
            "querySelectorAll",
            NativeFunction::from_fn_ptr(native_query_selector_all),
        ),
        (
            "createElement",
            NativeFunction::from_fn_ptr(native_create_element),
        ),
        (
            "createTextNode",
            NativeFunction::from_fn_ptr(native_create_text_node),
        ),
        (
            "createComment",
            NativeFunction::from_fn_ptr(native_create_comment),
        ),
        (
            "createDocumentFragment",
            NativeFunction::from_fn_ptr(native_create_document_fragment),
        ),
    ] {
        doc_obj.set(js_string!(name), js_fn(context, native), false, context)?;
    }
    for (name, native) in [
        (
            "documentElement",
            NativeFunction::from_fn_ptr(native_get_document_element),
        ),
        (
            "body",
            NativeFunction::from_fn_ptr(native_get_document_body),
        ),
        (
            "head",
            NativeFunction::from_fn_ptr(native_get_document_head),
        ),
        (
            "doctype",
            NativeFunction::from_fn_ptr(native_get_document_doctype),
        ),
    ] {
        let desc = PropertyDescriptorBuilder::new()
            .get(js_fn(context, native))
            .enumerable(true)
            .configurable(true)
            .build();
        doc_obj.define_property_or_throw(js_string!(name), desc, context)?;
    }
    context.register_global_property(js_string!("document"), document, Attribute::all())?;

    // console
    let console = {
        let mut console = ObjectInitializer::new(context);
        for (name, lvl) in [("log", 0u8), ("warn", 1), ("error", 2)] {
            let native = match lvl {
                0 => NativeFunction::from_fn_ptr(console_log),
                1 => NativeFunction::from_fn_ptr(console_warn),
                _ => NativeFunction::from_fn_ptr(console_error),
            };
            console.function(native, js_string!(name), 1);
        }
        console.build()
    };
    context.register_global_property(js_string!("console"), console, Attribute::all())?;

    // alert / quit
    context.register_global_builtin_callable(
        js_string!("alert"),
        1,
        NativeFunction::from_fn_ptr(native_alert),
    )?;
    context.register_global_builtin_callable(
        js_string!("quit"),
        0,
        NativeFunction::from_fn_ptr(native_quit),
    )?;

    // timers (scheduled on the script thread; see `ScriptEngine::run_due_timers`)
    for (name, arity, native) in [
        (
            "setTimeout",
            2usize,
            NativeFunction::from_fn_ptr(native_set_timeout),
        ),
        (
            "setInterval",
            2,
            NativeFunction::from_fn_ptr(native_set_interval),
        ),
        (
            "clearTimeout",
            1,
            NativeFunction::from_fn_ptr(native_clear_timeout),
        ),
        (
            "clearInterval",
            1,
            NativeFunction::from_fn_ptr(native_clear_interval),
        ),
    ] {
        context.register_global_builtin_callable(js_string!(name), arity, native)?;
    }

    // fetch (async resource load; see `ScriptEngine::poll_fetches`)
    context.register_global_builtin_callable(
        js_string!("fetch"),
        1,
        NativeFunction::from_fn_ptr(native_fetch),
    )?;
    Ok(())
}

/// JS `fetch(url)` — resolves through the unified resource layer.
///
/// Returns a `Promise` that fulfils with a `Response`-like object (`status`,
/// `ok`, `text()`). Completion is reported back to
/// [`ScriptEngine::poll_fetches`] (crate::script::ScriptEngine::poll_fetches),
/// which settles the promise and runs the microtask queue.
fn native_fetch(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let url = args
        .first()
        .map(|v| v.to_string(context))
        .transpose()?
        .unwrap_or_default();
    let url = url.to_std_string_escaped();
    if url.is_empty() {
        return Err(JsNativeError::typ()
            .with_message("fetch expects a non-empty URL")
            .into());
    }
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };

    // Create a pending promise. Its resolvers are GC-rooted in the
    // `__htuiFetchResolvers` JS registry (keyed by fetch id); keeping them in
    // Rust would leave them invisible to Boa's collector and they could be
    // reclaimed before the load completes.
    let (promise, resolvers) = JsPromise::new_pending(context);
    let (id, tx) = {
        let mut st = rc.borrow_mut();
        let id = st.next_fetch_id;
        st.next_fetch_id += 1;
        st.active_fetches.insert(id);
        (id, st.fetch_tx.clone())
    };
    let global = context.global_object();
    let registry = global
        .get(js_string!("__htuiFetchResolvers"), context)?
        .to_object(context)?;
    let mut entry = ObjectInitializer::new(context);
    entry.property(
        js_string!("resolve"),
        JsValue::from(resolvers.resolve.clone()),
        Attribute::all(),
    );
    entry.property(
        js_string!("reject"),
        JsValue::from(resolvers.reject.clone()),
        Attribute::all(),
    );
    registry.set(js_string!(id.to_string()), entry.build(), false, context)?;

    // Kick off the load. The result is tagged with the fetch id
    // so `poll_fetches` can route it back to the right promise. Relative
    // URLs resolve against the document base URL (`<base href>`) the same
    // way `<img src>` / `<script src>` do.
    let resolved = {
        let st = rc.borrow();
        let base = document_base_href(&st.arena, st.root_id);
        resources::resolve_url(base.as_deref(), &url)
    };
    resources::load_async(id, resolved, tx);
    Ok(promise.into())
}

/// Build a `Response`-like object for a fulfilled fetch: `status` (200), `ok`
/// (true), `url`, and a `text()` method returning a promise of the body as
/// UTF-8 text. The body bytes are captured in the closure, so `text()` can be
/// called repeatedly.
pub(crate) fn response_object(
    context: &mut Context,
    url: String,
    bytes: std::sync::Arc<Vec<u8>>,
) -> JsResult<JsValue> {
    let text = js_string!(String::from_utf8_lossy(bytes.as_ref()).into_owned());
    let text_native =
        |_this: &JsValue, _args: &[JsValue], captured: &JsString, ctx: &mut Context| {
            let (promise, resolvers) = JsPromise::new_pending(ctx);
            resolvers.resolve.call(
                &JsValue::undefined(),
                &[JsValue::from(captured.clone())],
                ctx,
            )?;
            Ok(promise.into())
        };
    let text_fn = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(text_native, text),
    );
    let mut init = ObjectInitializer::new(context);
    init.property(js_string!("status"), JsValue::from(200), Attribute::all());
    init.property(js_string!("ok"), JsValue::from(true), Attribute::all());
    init.property(
        js_string!("url"),
        JsValue::from(js_string!(url)),
        Attribute::all(),
    );
    init.property(js_string!("text"), text_fn, Attribute::all());
    Ok(init.build().into())
}

fn native_set_timeout(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    schedule_timer(context, args, false)
}

fn native_set_interval(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    schedule_timer(context, args, true)
}

/// Register `args[0]` (a callable) to fire after `args[1]` ms, returning the
/// timer id. `repeat` selects `setInterval` (re-scheduled after each fire)
/// vs `setTimeout` (one-shot). The callback is GC-rooted in the
/// `__htuiTimers` JS registry; the deadline lives in the Rust timer table the
/// script thread drives.
fn schedule_timer(context: &mut Context, args: &[JsValue], repeat: bool) -> JsResult<JsValue> {
    let callback = args.first().cloned().unwrap_or(JsValue::undefined());
    if !callback.is_callable() {
        return Err(JsNativeError::typ()
            .with_message("setTimeout/setInterval expects a callable first argument")
            .into());
    }
    let ms = args
        .get(1)
        .map(|v| v.to_number(context))
        .transpose()?
        .unwrap_or(0.0);
    let ms = if ms.is_finite() && ms > 0.0 { ms } else { 0.0 };
    let period = Duration::from_millis(ms as u64);
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let id = {
        let mut st = rc.borrow_mut();
        let id = st.next_timer_id;
        st.next_timer_id += 1;
        st.timers.push(TimerEntry {
            id,
            deadline: Instant::now() + period,
            period,
            repeat,
        });
        id
    };
    // GC-root the callback in the JS registry (state borrow already released).
    let global = context.global_object();
    let registry = global
        .get(js_string!("__htuiTimers"), context)?
        .to_object(context)?;
    registry.set(js_string!(id.to_string()), callback, false, context)?;
    Ok(id.into())
}

fn native_clear_timeout(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    clear_timer(context, args)
}

fn native_clear_interval(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    clear_timer(context, args)
}

/// Remove the timer with `args[0]`'s id from both the Rust table and the JS
/// registry, if present.
fn clear_timer(context: &mut Context, args: &[JsValue]) -> JsResult<JsValue> {
    let id = args
        .first()
        .map(|v| v.to_number(context))
        .transpose()?
        .unwrap_or(-1.0);
    let id = if id.is_finite() && id >= 0.0 {
        id as u32
    } else {
        u32::MAX
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    rc.borrow_mut().timers.retain(|t| t.id != id);
    let global = context.global_object();
    if let Ok(registry) = global
        .get(js_string!("__htuiTimers"), context)
        .and_then(|v| v.to_object(context))
        && let Err(e) = registry.delete_property_or_throw(js_string!(id.to_string()), context)
    {
        log::warn!("Failed to clear timer callback: {e}");
    }
    Ok(JsValue::undefined())
}

fn native_get_element_by_id(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let id = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?;
    let Some(id) = id else {
        return Ok(JsValue::null());
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let found = st
        .arena
        .iter()
        .find(|(_, n)| n.attributes.get("id").map(String::as_str) == Some(id.as_str()))
        .map(|(nid, _)| nid);
    match found {
        Some(node_id) => node_wrapper(context, node_id),
        None => Ok(JsValue::null()),
    }
}

fn native_create_text_node(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let data = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    // Detached text node (renders nothing until appended).
    let text_id = st.arena.insert(Node::text(&data));
    drop(st);
    node_wrapper(context, text_id)
}

fn native_create_comment(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let data = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    let comment_id = st.arena.insert(Node::comment(&data));
    drop(st);
    node_wrapper(context, comment_id)
}

fn native_create_document_fragment(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    let fragment_id = st.arena.insert(Node::fragment());
    drop(st);
    node_wrapper(context, fragment_id)
}

/// Find the first `<html>` descendant of the document root (`documentElement`).
fn native_get_document_element(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    find_descendant_by_type(context, NodeType::Html)
}

/// Find the `<body>` element (`document.body`).
fn native_get_document_body(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    find_descendant_by_type(context, NodeType::Body)
}

/// Find the `<head>` element (`document.head`).
fn native_get_document_head(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    find_descendant_by_type(context, NodeType::Head)
}

/// Find the `<!DOCTYPE>` node (`document.doctype`).
fn native_get_document_doctype(
    _this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    find_descendant_by_type(context, NodeType::DocumentType)
}

/// Depth-first search for the first node of `node_type` under the root.
fn find_descendant_by_type(context: &mut Context, node_type: NodeType) -> JsResult<JsValue> {
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let found = Node::descendants(&st.arena, st.root_id)
        .find(|(_, n)| n.node_type == node_type)
        .map(|(id, _)| id);
    match found {
        Some(id) => node_wrapper(context, id),
        None => Ok(JsValue::null()),
    }
}

fn native_create_element(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let tag = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?;
    let tag = tag.unwrap_or_else(|| "div".to_string());
    let node_type = crate::html::html_parse::tag_name_to_node_type(&tag).unwrap_or(NodeType::Div);
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    let node_id = st.arena.insert(Node::from(node_type));
    // Not attached to the document yet.
    drop(st);
    node_wrapper(context, node_id)
}

fn console_log(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let msg = format_args(args, context)?;
    // `target: "console"` separates JS console output from engine log
    // records (hosts can filter on `record.target()`); the `[console.log]`
    // prefix keeps it readable in plain-text streams.
    log::info!(target: "console", "[console.log] {msg}");
    Ok(JsValue::undefined())
}

fn console_warn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let msg = format_args(args, context)?;
    log::warn!(target: "console", "[console.warn] {msg}");
    Ok(JsValue::undefined())
}

fn console_error(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let msg = format_args(args, context)?;
    log::error!(target: "console", "[console.error] {msg}");
    Ok(JsValue::undefined())
}

fn format_args(args: &[JsValue], context: &mut Context) -> JsResult<String> {
    let mut parts = Vec::new();
    for a in args {
        let s = match a {
            v if v.is_string() => v.to_string(context)?.to_std_string_lossy(),
            v if v.is_number() => v.to_number(context)?.to_string(),
            v if v.is_null() => "null".to_string(),
            v if v.is_undefined() => "undefined".to_string(),
            v if v.is_boolean() => v.to_boolean().to_string(),
            v => {
                let s = v.to_string(context)?.to_std_string_lossy();
                if s.is_empty() {
                    "[object Object]".to_string()
                } else {
                    s
                }
            }
        };
        parts.push(s);
    }
    Ok(parts.join(" "))
}

fn native_alert(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let msg = format_args(args, context)?;
    log::warn!("[alert] {msg}");
    Ok(JsValue::undefined())
}

fn native_quit(_this: &JsValue, _args: &[JsValue], _context: &mut Context) -> JsResult<JsValue> {
    if let Some(rc) = with_state() {
        let mut st = rc.borrow_mut();
        st.render_cmds.push(RenderCmd::Quit);
    }
    Ok(JsValue::undefined())
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
