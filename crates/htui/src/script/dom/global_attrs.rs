//! Global attribute natives: `className` / `classList` / `hidden` /
//! `dataset` (live objects over the element's attributes).

use boa_engine::{Context, JsResult, JsValue, NativeFunction, js_string};

use super::{f64_to_node_id, js_fn, mark_dirty, node_id_to_f64, this_node_id, with_state};

pub(super) fn native_get_class(
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
        .and_then(|n| n.attributes.get("class").cloned())
        .unwrap_or_default();
    Ok(js_string!(value).into())
}

pub(super) fn native_set_class(
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
pub(super) fn native_get_class_list(
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
pub(super) fn native_get_hidden(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
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
pub(super) fn native_set_hidden(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
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
pub(super) fn native_get_dataset(
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
