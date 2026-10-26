//! Event listener natives: `addEventListener` / `removeEventListener`, the
//! element focus requests, the `el.click()` synchronous dispatch and the
//! JS-side listener registry helpers.

use boa_engine::{
    Context, JsNativeError, JsResult, JsValue, NativeFunction, js_string,
    object::ObjectInitializer, property::Attribute,
};

use crate::{
    events::{EventType, HtmlEvent},
    tdom::node::{NodeId, NodeType},
    tui::graph_api::RenderCmd,
};

use super::{node_id_to_f64, node_wrapper, this_node_id, with_state};

// ============================================================
// Event object construction + el.click()
// ============================================================

/// Build the JS event object shared by every dispatch path (the engine's
/// bubbling dispatch and `el.click()`): `type` / `target` / `currentTarget`
/// / `x` / `y` / `bubbles` / `defaultPrevented`, the hidden control flags
/// the dispatch loop reads, and the `stopPropagation` /
/// `stopImmediatePropagation` / `preventDefault` methods. Keyboard fields
/// are appended by the engine's key dispatch, which has a
/// [`KeyEventInfo`](crate::events::KeyEventInfo) payload.
pub(crate) fn make_event_object(
    context: &mut Context,
    event_type: EventType,
    target: &JsValue,
    x: u32,
    y: u32,
) -> JsResult<JsValue> {
    let type_str = event_type_str(event_type);
    let mut init = ObjectInitializer::new(context);
    init.property(js_string!("type"), js_string!(type_str), Attribute::all())
        .property(js_string!("target"), target.clone(), Attribute::all())
        .property(
            js_string!("currentTarget"),
            JsValue::null(),
            Attribute::all(),
        )
        .property(js_string!("x"), x as f64, Attribute::all())
        .property(js_string!("y"), y as f64, Attribute::all())
        .property(js_string!("bubbles"), true, Attribute::all())
        .property(js_string!("defaultPrevented"), false, Attribute::all())
        .property(
            js_string!("_stopped"),
            false,
            Attribute::WRITABLE | Attribute::NON_ENUMERABLE,
        )
        .property(
            js_string!("_stoppedImmediate"),
            false,
            Attribute::WRITABLE | Attribute::NON_ENUMERABLE,
        )
        .property(
            js_string!("_prevented"),
            false,
            Attribute::WRITABLE | Attribute::NON_ENUMERABLE,
        )
        .function(
            NativeFunction::from_fn_ptr(crate::script::bindings::stop_propagation),
            js_string!("stopPropagation"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(crate::script::bindings::stop_immediate_propagation),
            js_string!("stopImmediatePropagation"),
            0,
        )
        .function(
            NativeFunction::from_fn_ptr(crate::script::bindings::prevent_default),
            js_string!("preventDefault"),
            0,
        );
    Ok(init.build().into())
}

/// Read a boolean control flag off an event object.
fn event_flag(context: &mut Context, event: &JsValue, name: &str) -> bool {
    event
        .as_object()
        .and_then(|obj| obj.get(js_string!(name), context).ok())
        .is_some_and(|v| v.is_boolean() && v.to_boolean())
}

/// The disable-able form controls — `el.click()` on a disabled one fires
/// nothing (browser behavior; matches the pointer path's disabled gate).
fn is_form_control(node_type: NodeType) -> bool {
    matches!(
        node_type,
        NodeType::Input
            | NodeType::Textarea
            | NodeType::Select
            | NodeType::Button
            | NodeType::Optgroup
            | NodeType::HtmlOption
            | NodeType::Fieldset
    )
}

/// `el.click()` — synchronously dispatch a `click` event on this element
/// along its DOM ancestor chain, honouring `stopPropagation` /
/// `stopImmediatePropagation`, exactly like the engine's own bubbling
/// dispatch. Renders commands host functions push during the dispatch stay
/// in the session's render queue and travel with the next response. No
/// default actions run (no form submit / checkbox toggle) — JS listeners
/// only, the documented htui approximation.
pub(super) fn native_click(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };

    // Bubbling path: target first, then ancestors (like `region_for`).
    let (path, target_id_attr) = {
        let st = rc.borrow();
        let Some(node) = st.arena.get(node_id) else {
            return Ok(JsValue::undefined());
        };
        if is_form_control(node.node_type) && node.attributes.contains_key("disabled") {
            return Ok(JsValue::undefined());
        }
        let target_id_attr = node.get_id().cloned();
        let mut path = vec![node_id];
        let mut cur = node.parent;
        while let Some(id) = cur
            && let Some(n) = st.arena.get(id)
        {
            path.push(id);
            cur = n.parent;
        }
        (path, target_id_attr)
    };

    let target = node_wrapper(context, node_id)?;
    let event = make_event_object(context, EventType::Click, &target, 0, 0)?;

    // Host functions running inside a listener read the in-flight event
    // through `ScriptState.current_event` — populate it for this dispatch.
    {
        let mut st = rc.borrow_mut();
        st.current_event = Some(HtmlEvent {
            event_type: EventType::Click,
            target: target_id_attr,
            current_target: None,
            x: 0,
            y: 0,
            bubbles: true,
            key: None,
        });
    }

    for &element_id in &path {
        let Ok(wrapper) = node_wrapper(context, element_id) else {
            continue;
        };
        if let Some(obj) = event.as_object()
            && let Err(e) = obj.set(js_string!("currentTarget"), wrapper.clone(), false, context)
        {
            log::warn!("Failed to set event.currentTarget: {e}");
        }
        {
            let mut st = rc.borrow_mut();
            let current_target = st.arena.get(element_id).and_then(|n| n.get_id().cloned());
            if let Some(ev) = st.current_event.as_mut() {
                ev.current_target = current_target;
            }
        }

        let listeners = get_listeners(context, element_id, EventType::Click).unwrap_or_default();
        let mut immediate = false;
        for listener in listeners {
            if let Ok(f) = listener.to_object(context)
                && let Err(e) = f.call(&wrapper, std::slice::from_ref(&event), context)
            {
                log::warn!("click() listener threw: {e}");
            }
            if event_flag(context, &event, "_stoppedImmediate") {
                immediate = true;
                break;
            }
        }
        if !immediate
            && let Ok(Some(handler)) = get_attr_handler(context, element_id, EventType::Click)
        {
            if let Ok(f) = handler.to_object(context)
                && let Err(e) = f.call(&wrapper, std::slice::from_ref(&event), context)
            {
                log::warn!("click() attribute handler threw: {e}");
            }
            if event_flag(context, &event, "_stoppedImmediate") {
                immediate = true;
            }
        }
        if immediate || event_flag(context, &event, "_stopped") {
            break;
        }
    }

    let mut st = rc.borrow_mut();
    st.current_event = None;
    Ok(JsValue::undefined())
}

pub(super) fn native_add_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(type_name) = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
    else {
        return Err(JsNativeError::typ()
            .with_message("missing event type")
            .into());
    };
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
    // Known engine types register through the typed path (the engine's own
    // dispatch fires them); any other string registers verbatim so
    // `dispatchEvent` on a CustomEvent type reaches its listeners, like a
    // browser's open event-type namespace.
    match parse_event_type(args.first(), context) {
        Ok(event_type) => add_listener(context, node_id, event_type, fn_val)?,
        Err(_) => add_listener_by_name(context, node_id, &type_name, fn_val)?,
    }
    Ok(JsValue::undefined())
}

pub(super) fn native_remove_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let type_name = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?;
    match parse_event_type(args.first(), context) {
        Ok(event_type) => remove_listener(context, node_id, event_type, args.get(1))?,
        Err(_) => {
            if let Some(name) = type_name {
                remove_listener_by_name(context, node_id, &name, args.get(1))?;
            }
        }
    }
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
pub(super) fn native_focus(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
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
pub(super) fn native_blur(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    st.render_cmds.push(RenderCmd::BlurRequest { node_id });
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

/// Install (overwrite) the `on*` attribute handler for `(node, event_type)`
/// in the single-slot registry. Browser semantics: an event handler is one
/// slot per element and type — the latest compilation replaces any previous
/// one, so re-compiling after `setAttribute` cannot double-fire.
pub(crate) fn set_attr_handler(
    context: &mut Context,
    node_id: NodeId,
    event_type: EventType,
    handler: &JsValue,
) -> JsResult<()> {
    call_registry_helper(
        context,
        "__htuiSetAttrHandler",
        &[
            js_string!(node_id_to_f64(node_id).to_string()).into(),
            js_string!(event_type_str(event_type)).into(),
            handler.clone(),
        ],
    )
}

/// Clear the attribute-handler slot (browser `removeAttribute('onclick')`
/// semantics: the unwired handler no longer runs).
pub(crate) fn clear_attr_handler(
    context: &mut Context,
    node_id: NodeId,
    event_type: EventType,
) -> JsResult<()> {
    call_registry_helper(
        context,
        "__htuiClearAttrHandler",
        &[
            js_string!(node_id_to_f64(node_id).to_string()).into(),
            js_string!(event_type_str(event_type)).into(),
        ],
    )
}

/// Clear every attribute-handler slot of one node (all event types) — used
/// when a node leaves the document, so the registry cannot grow without
/// bound.
pub(crate) fn clear_node_attr_handler(context: &mut Context, node_id: NodeId) -> JsResult<()> {
    call_registry_helper(
        context,
        "__htuiClearAllAttrHandlers",
        &[js_string!(node_id_to_f64(node_id).to_string()).into()],
    )
}

/// Drop every multicast `addEventListener` listener of one node
pub(crate) fn clear_node_listeners(context: &mut Context, node_id: NodeId) -> JsResult<()> {
    call_registry_helper(
        context,
        "__htuiClearAllListeners",
        &[js_string!(node_id_to_f64(node_id).to_string()).into()],
    )
}

/// Read the current attribute-handler slot for `(node, event_type)`; `None`
/// when unset.
pub(crate) fn get_attr_handler(
    context: &mut Context,
    node_id: NodeId,
    event_type: EventType,
) -> JsResult<Option<JsValue>> {
    let global = context.global_object();
    let registry = global
        .get(js_string!("__htuiAttrHandlers"), context)?
        .to_object(context)?;
    let node_entry = registry.get(js_string!(node_id_to_f64(node_id).to_string()), context)?;
    if node_entry.is_undefined() || node_entry.is_null() {
        return Ok(None);
    }
    let node_obj = node_entry.to_object(context)?;
    let handler = node_obj.get(js_string!(event_type_str(event_type)), context)?;
    Ok((!handler.is_undefined() && !handler.is_null()).then_some(handler))
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
    /// Capacity hint ceiling for the pre-allocation below. `length` is read
    /// from a JS-owned array object (page JS can reach the helper and tamper
    /// with its registries), so a fabricated huge length must not translate
    /// into a huge `with_capacity` allocation — the push loop below grows the
    /// vector normally for whatever elements actually exist.
    const CAPACITY_HINT_CEILING: u32 = 4096;

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
    let mut out = Vec::with_capacity(len.min(CAPACITY_HINT_CEILING) as usize);
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
// Name-keyed listeners (CustomEvent types) + dispatchEvent
// ============================================================

/// Register a listener under an arbitrary type name (the same registry the
/// typed path uses — known types share one key space, so an engine dispatch
/// of e.g. `click` reaches a by-name `click` registration too).
pub(super) fn add_listener_by_name(
    context: &mut Context,
    node_id: NodeId,
    type_name: &str,
    listener: &JsValue,
) -> JsResult<()> {
    call_registry_helper(
        context,
        "__htuiAddListener",
        &[
            js_string!(node_id_to_f64(node_id).to_string()).into(),
            js_string!(type_name).into(),
            listener.clone(),
        ],
    )
}

/// Remove a listener registered under an arbitrary type name (identity
/// match, mirroring [`remove_listener`]).
pub(super) fn remove_listener_by_name(
    context: &mut Context,
    node_id: NodeId,
    type_name: &str,
    listener: Option<&JsValue>,
) -> JsResult<()> {
    let key = node_id_to_f64(node_id).to_string();
    let global = context.global_object();
    let registry = global
        .get(js_string!("__htuiListeners"), context)?
        .to_object(context)?;
    let node_entry = registry.get(js_string!(key), context)?;
    if node_entry.is_undefined() {
        return Ok(());
    }
    let node_obj = node_entry.to_object(context)?;
    let arr = node_obj.get(js_string!(type_name), context)?;
    if arr.is_undefined() {
        return Ok(());
    }
    let Some(listener) = listener else {
        // No listener given: drop the whole type slot.
        node_obj.delete_property_or_throw(js_string!(type_name), context)?;
        return Ok(());
    };
    let arr_obj = arr.to_object(context)?;
    let len = arr_obj
        .get(js_string!("length"), context)?
        .to_u32(context)?;
    let mut keep = Vec::new();
    for i in 0..len {
        let item = arr_obj.get(i, context)?;
        if !item.strict_equals(listener) {
            keep.push(item);
        }
    }
    // Rewrite in place: truncate first so removed tail entries cannot
    // linger past the new length.
    arr_obj.set(js_string!("length"), JsValue::from(0u32), false, context)?;
    for (idx, item) in keep.into_iter().enumerate() {
        arr_obj.set(idx, item, false, context)?;
    }
    Ok(())
}

/// Fetch the listener array registered under an arbitrary type name (empty
/// when none).
pub(super) fn get_listeners_by_name(
    context: &mut Context,
    node_id: NodeId,
    type_name: &str,
) -> JsResult<Vec<JsValue>> {
    let global = context.global_object();
    let helper = global
        .get(js_string!("__htuiGetListeners"), context)?
        .to_object(context)?;
    let arr = helper.call(
        &JsValue::undefined(),
        &[
            js_string!(node_id_to_f64(node_id).to_string()).into(),
            js_string!(type_name).into(),
        ],
        context,
    )?;
    let arr_obj = arr.to_object(context)?;
    let len = arr_obj
        .get(js_string!("length"), context)?
        .to_u32(context)?;
    let mut out = Vec::with_capacity(len.min(1024) as usize);
    for i in 0..len {
        out.push(arr_obj.get(i, context)?);
    }
    Ok(out)
}

/// `target.dispatchEvent(event)` — synchronously dispatch a JS-built event
/// (from the `Event` / `CustomEvent` constructors, or any object with a
/// `type` string) along the target's ancestor chain, honouring
/// `event.bubbles === false` (target-only) and the stop flags. Returns
/// `false` only when a listener called `preventDefault()` on a cancelable
/// event, like the DOM.
pub(super) fn native_dispatch_event(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let event = args.first().cloned().unwrap_or_default();
    let event_obj = event
        .to_object(context)
        .map_err(|_| JsNativeError::typ().with_message("dispatchEvent expects an Event object"))?;
    let type_name = event_obj
        .get(js_string!("type"), context)?
        .to_string(context)?
        .to_std_string_lossy();
    if type_name.is_empty() {
        return Err(JsNativeError::typ()
            .with_message("dispatchEvent expects a non-empty event type")
            .into());
    }
    let bubbles = event_obj
        .get(js_string!("bubbles"), context)
        .map(|v| v.is_boolean() && v.to_boolean())
        .unwrap_or(false);
    let Some(rc) = with_state() else {
        return Ok(JsValue::from(true));
    };
    // Path: target first, then ancestors (bubbling); target-only when the
    // event says it does not bubble.
    let path = {
        let st = rc.borrow();
        let mut path = vec![node_id];
        if bubbles {
            let mut cur = st.arena.get(node_id).and_then(|n| n.parent);
            while let Some(id) = cur
                && let Some(n) = st.arena.get(id)
            {
                path.push(id);
                cur = n.parent;
            }
        }
        path
    };
    // The DOM sets `target` once at dispatch start (it never changes while
    // bubbling; only `currentTarget` follows the path).
    let target_wrapper = node_wrapper(context, node_id)?;
    if let Err(e) = event_obj.set(js_string!("target"), target_wrapper, false, context) {
        log::warn!("Failed to set event.target: {e}");
    }
    for &element_id in &path {
        let Ok(wrapper) = node_wrapper(context, element_id) else {
            continue;
        };
        if let Err(e) = event_obj.set(js_string!("currentTarget"), wrapper.clone(), false, context)
        {
            log::warn!("Failed to set event.currentTarget: {e}");
        }
        let listeners = get_listeners_by_name(context, element_id, &type_name)?;
        let mut immediate = false;
        for listener in listeners {
            if let Ok(f) = listener.to_object(context)
                && let Err(e) = f.call(&wrapper, std::slice::from_ref(&event), context)
            {
                log::warn!("dispatchEvent listener threw: {e}");
            }
            if event_flag(context, &event, "_stoppedImmediate") {
                immediate = true;
                break;
            }
        }
        if immediate || event_flag(context, &event, "_stopped") {
            break;
        }
    }
    let not_prevented = !event_flag(context, &event, "_prevented");
    Ok(JsValue::from(not_prevented))
}

// ============================================================
// Event / CustomEvent constructors
// ============================================================

/// `new Event(type, init?)` — build a plain event object with the control
/// flags and stop / prevent methods the dispatch loop reads. `init.bubbles`
/// / `init.cancelable` default to `false` (the DOM defaults); the flags are
/// still written so `preventDefault()` on a non-cancelable event is the
/// documented no-op (the dispatch result ignores `_prevented` there —
/// simplified: cancelability is not tracked separately).
pub(super) fn event_constructor(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let type_name = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let (bubbles, cancelable) = read_event_init(args.get(1), context)?;
    build_event_object(
        this,
        context,
        &type_name,
        bubbles,
        cancelable,
        JsValue::undefined(),
    )
}

/// `new CustomEvent(type, init?)` — an `Event` carrying `init.detail`.
pub(super) fn custom_event_constructor(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let type_name = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let (bubbles, cancelable) = read_event_init(args.get(1), context)?;
    let detail = args
        .get(1)
        .and_then(|v| v.as_object())
        .map(|obj| obj.get(js_string!("detail"), context))
        .transpose()?
        .unwrap_or(JsValue::undefined());
    build_event_object(this, context, &type_name, bubbles, cancelable, detail)
}

/// Read `{ bubbles, cancelable }` off an init object (both default false).
fn read_event_init(init: Option<&JsValue>, context: &mut Context) -> JsResult<(bool, bool)> {
    let Some(obj) = init.and_then(|v| v.as_object()) else {
        return Ok((false, false));
    };
    let bubbles = obj
        .get(js_string!("bubbles"), context)
        .map(|v| v.is_boolean() && v.to_boolean())
        .unwrap_or(false);
    let cancelable = obj
        .get(js_string!("cancelable"), context)
        .map(|v| v.is_boolean() && v.to_boolean())
        .unwrap_or(false);
    Ok((bubbles, cancelable))
}

/// Populate `this` (the freshly-constructed object under `new`) with the
/// event surface and return it.
/// `event.timeStamp` source: milliseconds since the Unix epoch (the DOM
/// spec allows a monotonic origin; wall-clock keeps it simple here).
fn now_millis() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn build_event_object(
    this: &JsValue,
    context: &mut Context,
    type_name: &str,
    bubbles: bool,
    cancelable: bool,
    detail: JsValue,
) -> JsResult<JsValue> {
    let obj = this
        .to_object(context)
        .map_err(|_| JsNativeError::typ().with_message("Event requires `new`"))?;
    obj.set(js_string!("type"), js_string!(type_name), false, context)?;
    obj.set(
        js_string!("bubbles"),
        JsValue::from(bubbles),
        false,
        context,
    )?;
    obj.set(
        js_string!("cancelable"),
        JsValue::from(cancelable),
        false,
        context,
    )?;
    obj.set(js_string!("detail"), detail, false, context)?;
    obj.set(js_string!("target"), JsValue::null(), false, context)?;
    obj.set(js_string!("currentTarget"), JsValue::null(), false, context)?;
    obj.set(
        js_string!("timeStamp"),
        JsValue::from(now_millis()),
        false,
        context,
    )?;
    obj.set(
        js_string!("defaultPrevented"),
        JsValue::from(false),
        false,
        context,
    )?;
    obj.set(js_string!("_stopped"), JsValue::from(false), false, context)?;
    obj.set(
        js_string!("_stoppedImmediate"),
        JsValue::from(false),
        false,
        context,
    )?;
    obj.set(
        js_string!("_prevented"),
        JsValue::from(false),
        false,
        context,
    )?;
    for (name, native) in [
        (
            "stopPropagation",
            crate::script::bindings::stop_propagation
                as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
        ),
        (
            "stopImmediatePropagation",
            crate::script::bindings::stop_immediate_propagation
                as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
        ),
        (
            "preventDefault",
            crate::script::bindings::prevent_default
                as fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue>,
        ),
    ] {
        let f = super::js_fn(context, NativeFunction::from_fn_ptr(native));
        let f_val: JsValue = f.into();
        obj.set(js_string!(name), f_val, false, context)?;
    }
    Ok(this.clone())
}
