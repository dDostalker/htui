//! Attribute natives: `getAttribute` / `setAttribute` / `removeAttribute`
//! / `hasAttribute` / `toggleAttribute` and the `id` property accessors.

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

/// Parse the attribute-name argument of an attribute native (lowercased,
/// like HTML attribute names). Missing or non-string input is a TypeError.
fn attr_name_arg(args: &[JsValue], method: &str) -> JsResult<String> {
    args.first()
        .and_then(|v| v.as_string())
        .map(|s| s.to_std_string_lossy().to_ascii_lowercase())
        .ok_or_else(|| {
            JsNativeError::typ()
                .with_message(format!("{method} expects a string"))
                .into()
        })
}

/// `element.hasAttribute(name)` — whether the attribute is present at all
/// (an empty value still counts).
pub(super) fn native_has_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = attr_name_arg(args, "hasAttribute")?;
    let has = with_state()
        .map(|rc| {
            rc.borrow()
                .arena
                .get(node_id)
                .map(|n| n.attributes.contains_key(&name))
                .unwrap_or(false)
        })
        .unwrap_or(false);
    Ok(JsValue::from(has))
}

/// `element.toggleAttribute(name, force?)` — flip the attribute's presence.
/// With `force`, add / remove unconditionally; without it, add when absent
/// and remove when present. Returns whether the attribute is present after
/// the call. Toggling an `on*` attribute (un)wires its handler, mirroring
/// `setAttribute` / `removeAttribute`.
pub(super) fn native_toggle_attribute(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = attr_name_arg(args, "toggleAttribute")?;
    let force = args.get(1).and_then(|v| v.as_boolean());
    let Some(rc) = with_state() else {
        return Ok(JsValue::from(false));
    };
    let added;
    {
        let mut st = rc.borrow_mut();
        let Some(node) = st.arena.get_mut(node_id) else {
            return Ok(JsValue::from(false));
        };
        let present = node.attributes.contains_key(&name);
        added = force.unwrap_or(!present);
        if added == present {
            return Ok(JsValue::from(present));
        }
        if added {
            node.attributes.insert(name.clone(), String::new());
        } else {
            node.attributes.shift_remove(&name);
        }
        mark_dirty(&mut st, node_id);
    }
    match crate::script::bindings::attribute_event_type(&name) {
        Some(_) if added => {
            crate::script::bindings::compile_node_attribute_listeners(context, &rc, node_id)?;
        }
        Some(event_type) => {
            super::events::clear_attr_handler(context, node_id, event_type)?;
        }
        None => {}
    }
    Ok(JsValue::from(added))
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
