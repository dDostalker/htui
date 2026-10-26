//! Attribute natives: `getAttribute` / `setAttribute` / `removeAttribute`
//! / `hasAttribute` / `toggleAttribute`, the `attributes` snapshot and the
//! `id` property accessors.

use boa_engine::{
    Context, JsNativeError, JsResult, JsValue, js_string,
    object::{JsObject, ObjectInitializer},
    property::Attribute,
};

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

/// `element.attributes` — a **static snapshot** of the attribute list in
/// source order (the live `NamedNodeMap` is not modelled): numeric indices
/// `0..length` yield `{ name, value }` records, plus a `length` and a
/// `getNamedItem(name)` helper built without captures (it reads the object
/// itself). An HTML attribute literally named `length` would shadow the
/// count — the documented approximation.
pub(super) fn native_get_attributes(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let st = rc.borrow();
    let Some(node) = st.arena.get(node_id) else {
        return Ok(JsValue::undefined());
    };
    let entries: Vec<(String, String)> = node
        .attributes
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    drop(st);

    let get_named_item = super::js_fn(
        context,
        boa_engine::NativeFunction::from_fn_ptr(named_item_lookup),
    );
    // Build the records before the `ObjectInitializer` takes its
    // `&mut Context` borrow.
    let mut records = Vec::with_capacity(entries.len());
    for (name, value) in entries.iter() {
        records.push((
            attr_record(context, name, value)?,
            js_string!(value.as_str()),
        ));
    }
    let mut init = ObjectInitializer::new(context);
    init.property(js_string!("length"), entries.len() as u32, Attribute::all());
    init.property(js_string!("getNamedItem"), get_named_item, Attribute::all());
    for (idx, (record, value_str)) in records.into_iter().enumerate() {
        let name = &entries[idx].0;
        init.property(idx, record, Attribute::all());
        // The named form reads as the attribute's value string.
        init.property(js_string!(name.as_str()), value_str, Attribute::all());
    }
    Ok(init.build().into())
}

/// `{ name, value }` record for one attribute.
fn attr_record(context: &mut Context, name: &str, value: &str) -> JsResult<JsObject> {
    let mut rec = ObjectInitializer::new(context);
    rec.property(js_string!("name"), js_string!(name), Attribute::all());
    rec.property(js_string!("value"), js_string!(value), Attribute::all());
    Ok(rec.build())
}

/// `attributes.getNamedItem(name)` — reads the `name` property off the
/// snapshot object (so the function needs no captured state). Returns the
/// `{ name, value }` record for a present attribute, `null` otherwise.
fn named_item_lookup(this: &JsValue, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let obj = this
        .to_object(context)
        .map_err(|_| JsNativeError::typ().with_message("expected an attributes object"))?;
    let name = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default()
        .to_ascii_lowercase();
    let length = obj.get(js_string!("length"), context)?.to_u32(context)?;
    for i in 0..length {
        let record = obj.get(i, context)?;
        if let Some(rec) = record.as_object() {
            let rec_name = rec
                .get(js_string!("name"), context)?
                .to_string(context)?
                .to_std_string_lossy()
                .to_ascii_lowercase();
            if rec_name == name {
                return Ok(record);
            }
        }
    }
    Ok(JsValue::null())
}
