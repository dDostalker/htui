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

use boa_engine::{
    Context, JsNativeError, JsObject, JsResult, JsString, JsValue, NativeFunction, js_string,
    object::{FunctionObjectBuilder, ObjectInitializer, builtins::JsFunction},
    property::Attribute,
};
use slotmap::Key;

use crate::{
    events::EventType,
    html::css_parse::parse_declaration_text,
    tdom::node::{Node, NodeArena, NodeId, NodeType},
    tui::graph_api::RenderCmd,
};

use super::ScriptState;

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

/// Create (or return the cached) element wrapper for `node_id`.
///
/// Wrappers are cached on the JS side (`globalThis.__htuiElements[nodeId]`) so
/// `el === event.target` identity checks work.
pub(crate) fn element_wrapper(context: &mut Context, node_id: NodeId) -> JsResult<JsValue> {
    let key = node_id_to_f64(node_id);
    let registry = registry_elements(context)?;
    let cached = registry.get(key, context)?;
    if !cached.is_undefined() {
        return Ok(cached);
    }
    let wrapper = make_element_wrapper(context, node_id)?;
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

/// Build a fresh element wrapper object for `node_id`.
fn make_element_wrapper(context: &mut Context, node_id: NodeId) -> JsResult<JsValue> {
    let node_key = node_id_to_f64(node_id);
    let hidden = Attribute::READONLY | Attribute::NON_ENUMERABLE;

    // Accessor functions must be built before the `ObjectInitializer` takes
    // its `&mut Context` borrow.
    let get_text = js_fn(context, NativeFunction::from_fn_ptr(native_get_text));
    let set_text = js_fn(context, NativeFunction::from_fn_ptr(native_set_text));
    let get_class = js_fn(context, NativeFunction::from_fn_ptr(native_get_class));
    let set_class = js_fn(context, NativeFunction::from_fn_ptr(native_set_class));
    let get_id = js_fn(context, NativeFunction::from_fn_ptr(native_get_id));
    let set_id = js_fn(context, NativeFunction::from_fn_ptr(native_set_id));
    let get_parent = js_fn(context, NativeFunction::from_fn_ptr(native_get_parent));
    let get_style = js_fn(context, NativeFunction::from_fn_ptr(native_get_style));

    let mut init = ObjectInitializer::new(context);
    init.property(js_string!("_nodeId"), node_key, hidden);

    // Methods
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
        NativeFunction::from_fn_ptr(native_add_event_listener),
        js_string!("addEventListener"),
        2,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_remove_event_listener),
        js_string!("removeEventListener"),
        2,
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

    // Accessors
    init.accessor(js_string!("textContent"), Some(get_text), Some(set_text), Attribute::all());
    init.accessor(js_string!("className"), Some(get_class), Some(set_class), Attribute::all());
    init.accessor(js_string!("id"), Some(get_id), Some(set_id), Attribute::all());
    init.accessor(js_string!("parentNode"), Some(get_parent), None, Attribute::all());
    init.accessor(js_string!("style"), Some(get_style), None, Attribute::all());
    let obj = init.build();
    Ok(obj.into())
}

// ============================================================
// Element natives
// ============================================================

fn native_get_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = args.get(0).and_then(|v| v.as_string()).map(|s| s.to_std_string_lossy());
    let Some(name) = name else {
        return Err(JsNativeError::typ().with_message("getAttribute expects a string").into());
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let value = st
        .arena
        .get(node_id)
        .and_then(|n| n.attributes.get(&name).cloned());
    Ok(value.map(|s| js_string!(s).into()).unwrap_or(JsValue::null()))
}

fn native_set_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = args
        .get(0)
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy());
    let value = args
        .get(1)
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy());
    let (Some(name), Some(value)) = (name, value) else {
        return Err(JsNativeError::typ().with_message("setAttribute expects two strings").into());
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
    let name = args.get(0).and_then(|v| v.as_string()).map(|s| s.to_std_string_lossy());
    let Some(name) = name else {
        return Err(JsNativeError::typ().with_message("removeAttribute expects a string").into());
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    if let Some(node) = st.arena.get_mut(node_id) {
        node.attributes.remove(&name);
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
        .get(0)
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();

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
        .filter(|&c| matches!(st.arena.get(c).map(|n| &n.node_type), Some(NodeType::Text(_))));
    if let Some(child_id) = reuse_child {
        let existing = st.arena.get_mut(child_id).map(|c| &mut c.node_type);
        if let Some(NodeType::Text(existing)) = existing {
            if *existing == text {
                return Ok(JsValue::undefined());
            }
            *existing = text;
            mark_dirty(&mut st, node_id);
            return Ok(JsValue::undefined());
        }
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
    let text_id = st.arena.insert(Node::text(&text));
    if let Some(parent) = st.arena.get_mut(node_id) {
        parent.children.push(text_id);
    }
    if let Some(t) = st.arena.get_mut(text_id) {
        t.parent = Some(node_id);
    }
    mark_dirty(&mut st, node_id);
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
        .get(0)
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
        .get(0)
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

fn native_get_parent(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let st = rc.borrow();
    let parent = st.arena.get(node_id).and_then(|n| n.parent);
    match parent {
        Some(pid) => element_wrapper(context, pid),
        None => Ok(JsValue::null()),
    }
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
    for (camel, kebab) in STYLE_PROPS {
        let kebab_capture = *kebab;
        let getter = NativeFunction::from_copy_closure_with_captures(
            |this, _args, kebab: &&'static str, context| style_get(this, kebab, context),
            kebab_capture,
        );
        let setter = NativeFunction::from_copy_closure_with_captures(
            |this, args, kebab: &&'static str, context| {
                style_set(this, kebab, args.first(), context)
            },
            kebab_capture,
        );
        let get = js_fn(context, getter);
        let set = js_fn(context, setter);
        accessors.push((js_string!(*camel), Some(get), Some(set)));
    }
    // `cssText` gives access to the raw style attribute.
    let get_css = js_fn(context, NativeFunction::from_fn_ptr(style_get_css_text));
    let set_css = js_fn(context, NativeFunction::from_fn_ptr(style_set_css_text));

    let mut init = ObjectInitializer::new(context);
    init.property(js_string!("_nodeId"), node_key, hidden);
    for (name, get, set) in accessors {
        init.accessor(name, get, set, Attribute::all());
    }
    init.accessor(js_string!("cssText"), Some(get_css), Some(set_css), Attribute::all());
    Ok(init.build().into())
}

/// (camelCase JS property, kebab-case CSS property) pairs exposed on `el.style`.
const STYLE_PROPS: &[(&str, &str)] = &[
    ("color", "color"),
    ("backgroundColor", "background-color"),
    ("fontSize", "font-size"),
    ("fontWeight", "font-weight"),
    ("fontStyle", "font-style"),
    ("textAlign", "text-align"),
    ("textDecoration", "text-decoration"),
    ("textTransform", "text-transform"),
    ("lineHeight", "line-height"),
    ("display", "display"),
    ("width", "width"),
    ("height", "height"),
    ("padding", "padding"),
    ("paddingTop", "padding-top"),
    ("paddingRight", "padding-right"),
    ("paddingBottom", "padding-bottom"),
    ("paddingLeft", "padding-left"),
    ("margin", "margin"),
    ("marginTop", "margin-top"),
    ("marginRight", "margin-right"),
    ("marginBottom", "margin-bottom"),
    ("marginLeft", "margin-left"),
    ("border", "border"),
    ("borderWidth", "border-width"),
    ("borderStyle", "border-style"),
    ("borderColor", "border-color"),
    ("whiteSpace", "white-space"),
    ("flexDirection", "flex-direction"),
    ("flexWrap", "flex-wrap"),
    ("justifyContent", "justify-content"),
    ("alignItems", "align-items"),
    ("alignContent", "align-content"),
    ("gap", "gap"),
];

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

fn style_get_css_text(this: &JsValue, _args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
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

fn style_set_css_text(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let value = args
        .get(0)
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
        return Err(JsNativeError::typ().with_message("addEventListener expects a function").into());
    };
    if fn_val.as_object().is_none() || !fn_val.is_callable() {
        return Err(JsNativeError::typ().with_message("listener is not a function").into());
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
    if let Some(old_parent) = old_parent {
        if let Some(op) = st.arena.get_mut(old_parent) {
            op.children.retain(|c| *c != child_id);
        }
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
    }
}

pub(crate) fn parse_event_type(
    arg: Option<&JsValue>,
    context: &mut Context,
) -> JsResult<EventType> {
    let Some(arg) = arg else {
        return Err(JsNativeError::typ().with_message("missing event type").into());
    };
    let s = arg.to_string(context)?.to_std_string_lossy();
    match s.as_str() {
        "click" => Ok(EventType::Click),
        "mouseover" => Ok(EventType::MouseOver),
        "mouseout" => Ok(EventType::MouseOut),
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
    call_registry_helper(context, "__htuiAddListener", &[
        js_string!(node_id_to_f64(node_id).to_string()).into(),
        js_string!(event_type_str(event_type)).into(),
        listener.clone(),
    ])
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
    let registry = global.get(js_string!("__htuiListeners"), context)?.to_object(context)?;
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
    let len = arr_obj.get(js_string!("length"), context)?.to_u32(context)?;
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
            new_arr.into_iter().enumerate().fold(&mut init, |acc, (i, v)| {
                acc.property(i, v, Attribute::WRITABLE | Attribute::ENUMERABLE | Attribute::CONFIGURABLE);
                acc
            });
            init.build().into()
        };
        node_obj.set(js_string!(event_type_str(event_type)), new_js, false, context)?;
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
    let helper = global.get(js_string!("__htuiGetListeners"), context)?.to_object(context)?;
    let arr = helper.call(
        &JsValue::undefined(),
        &[
            js_string!(node_id_to_f64(node_id).to_string()).into(),
            js_string!(event_type_str(event_type)).into(),
        ],
        context,
    )?;
    let arr_obj = arr.to_object(context)?;
    let len = arr_obj.get(js_string!("length"), context)?.to_u32(context)?;
    let mut out = Vec::with_capacity(len as usize);
    for i in 0..len {
        out.push(arr_obj.get(i, context)?);
    }
    Ok(out)
}

/// Call a registry helper defined in the bootstrap script.
fn call_registry_helper(
    context: &mut Context,
    name: &str,
    args: &[JsValue],
) -> JsResult<()> {
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
    // document
    let mut doc = ObjectInitializer::new(context);
    doc.function(
        NativeFunction::from_fn_ptr(native_get_element_by_id),
        js_string!("getElementById"),
        1,
    );
    doc.function(
        NativeFunction::from_fn_ptr(native_create_element),
        js_string!("createElement"),
        1,
    );
    let document = doc.build();
    context.register_global_property(js_string!("document"), document, Attribute::all())?;

    // console
    let console = {
        let mut console = ObjectInitializer::new(context);
        for (name, lvl) in [
            ("log", 0u8),
            ("warn", 1),
            ("error", 2),
        ] {
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
    Ok(())
}

fn native_get_element_by_id(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let id = args.get(0).map(|v| v.to_string(context).map(|s| s.to_std_string_lossy())).transpose()?;
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
        Some(node_id) => element_wrapper(context, node_id),
        None => Ok(JsValue::null()),
    }
}

fn native_create_element(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let tag = args.get(0).map(|v| v.to_string(context).map(|s| s.to_std_string_lossy())).transpose()?;
    let tag = tag.unwrap_or_else(|| "div".to_string());
    let node_type = crate::html::html_parse::tag_name_to_node_type(&tag).unwrap_or(NodeType::Div);
    let Some(rc) = with_state() else {
        return Ok(JsValue::null());
    };
    let mut st = rc.borrow_mut();
    let node_id = st.arena.insert(Node::from(node_type));
    // Not attached to the document yet.
    element_wrapper(context, node_id)
}

fn console_log(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let msg = format_args(args, context)?;
    log::info!("[console.log] {msg}");
    Ok(JsValue::undefined())
}

fn console_warn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let msg = format_args(args, context)?;
    log::warn!("[console.warn] {msg}");
    Ok(JsValue::undefined())
}

fn console_error(_this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let msg = format_args(args, context)?;
    log::error!("[console.error] {msg}");
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
        if let NodeType::Text(s) = &node.node_type {
            out.push_str(s);
        }
        stack.extend(node.children.iter().rev().copied());
    }
    out
}
