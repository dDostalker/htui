//! Attribute natives: `getAttribute` / `setAttribute` / `removeAttribute`
//! and the `id` property accessors.

use boa_engine::{Context, JsNativeError, JsResult, JsValue, js_string};

use super::{mark_dirty, this_node_id, with_state};

pub(super) fn native_get_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = args
        .first()
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy().to_ascii_lowercase());
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

pub(super) fn native_set_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = args
        .first()
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy().to_ascii_lowercase());
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
    let wires_handler = crate::script::bindings::attribute_event_type(&name).is_some();
    {
        let mut st = rc.borrow_mut();
        if let Some(node) = st.arena.get_mut(node_id) {
            node.attributes.insert(name, value);
            mark_dirty(&mut st, node_id);
        }
    }
    // Browser semantics: wiring an `on*` attribute (re)compiles its handler
    // right away. The single-slot registry overwrites, so the latest
    // attribute value is authoritative and re-fires cannot accumulate.
    if wires_handler {
        crate::script::bindings::compile_node_attribute_listeners(context, &rc, node_id)?;
    }
    Ok(JsValue::undefined())
}

pub(super) fn native_remove_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = args
        .first()
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy().to_ascii_lowercase());
    let Some(name) = name else {
        return Err(JsNativeError::typ()
            .with_message("removeAttribute expects a string")
            .into());
    };
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let unwired_handler = crate::script::bindings::attribute_event_type(&name);
    {
        let mut st = rc.borrow_mut();
        if let Some(node) = st.arena.get_mut(node_id) {
            node.attributes.shift_remove(&name);
            mark_dirty(&mut st, node_id);
        }
    }
    // Browser semantics: removing an `on*` attribute unwires its handler.
    if let Some(event_type) = unwired_handler {
        super::events::clear_attr_handler(context, node_id, event_type)?;
    }
    Ok(JsValue::undefined())
}

pub(super) fn native_get_id(
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
        .and_then(|n| n.attributes.get("id").cloned())
        .unwrap_or_default();
    Ok(js_string!(value).into())
}

pub(super) fn native_set_id(
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
        node.attributes.insert("id".to_string(), value);
        mark_dirty(&mut st, node_id);
    }
    Ok(JsValue::undefined())
}
