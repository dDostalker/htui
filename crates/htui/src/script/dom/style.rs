//! `style` object natives: camelCase accessors over the element's `style`
//! attribute plus the raw `cssText` view.

use std::sync::Arc;

use boa_engine::{
    Context, JsResult, JsString, JsValue, NativeFunction, js_string,
    object::{ObjectInitializer, builtins::JsFunction},
    property::Attribute,
};

use crate::{
    html::css_parse::{SUPPORTED_PROPERTIES, parse_declaration_text},
    script::ScriptState,
    tdom::node::NodeId,
};

use super::{js_fn, mark_dirty, node_id_to_f64, this_node_id, with_state};

/// Return an object whose accessors read/write the element's `style`
/// attribute (CSS custom-property semantics, camelCase ↔ kebab-case).
pub(super) fn native_get_style(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
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
    init.function(
        NativeFunction::from_fn_ptr(native_set_property),
        js_string!("setProperty"),
        2,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_get_property_value),
        js_string!("getPropertyValue"),
        1,
    );
    init.function(
        NativeFunction::from_fn_ptr(native_remove_property),
        js_string!("removeProperty"),
        1,
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
fn style_map(st: &ScriptState, node_id: NodeId) -> indexmap::IndexMap<Arc<str>, Arc<str>> {
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
        .map(|v| v.as_ref().to_string())
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
        map.insert(Arc::from(*kebab), Arc::from(value));
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

/// CSSOM property-name normalization: regular properties lowercase;
/// custom properties (`--*`) are case-sensitive and stay as written.
fn normalize_property_name(name: &str) -> String {
    if name.starts_with("--") {
        name.to_string()
    } else {
        name.to_ascii_lowercase()
    }
}

/// Insert/remove `name` in the element's style attribute and serialize.
/// An empty `value` removes the declaration; returns whether the attribute
/// changed.
fn write_declaration(st: &mut ScriptState, node_id: NodeId, name: &str, value: &str) -> bool {
    let mut map = style_map(st, node_id);
    let changed = if value.is_empty() {
        map.shift_remove(name).is_some()
    } else {
        map.insert(Arc::from(name), Arc::from(value))
            .is_none_or(|old| old.as_ref() != value)
    };
    if changed && let Some(node) = st.arena.get_mut(node_id) {
        let serialized: Vec<String> = map.iter().map(|(k, v)| format!("{k}: {v}")).collect();
        node.attributes
            .insert("style".to_string(), serialized.join("; "));
        mark_dirty(st, node_id);
    }
    changed
}

/// `style.setProperty(name, value)` — writes any declaration, including
/// `--*` custom properties (case-sensitive); regular names lowercase. An
/// empty `value` removes the declaration. Unrecognized regular property
/// names are still stored (the cascade drops what it cannot parse, matching
/// the stylesheet path).
fn native_set_property(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
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
    let name = normalize_property_name(&name);
    if name.is_empty() {
        return Ok(JsValue::undefined());
    }
    if let Some(rc) = with_state() {
        let mut st = rc.borrow_mut();
        write_declaration(&mut st, node_id, &name, value.trim());
    }
    Ok(JsValue::undefined())
}

/// `style.getPropertyValue(name)` — the current declaration value (trimmed
/// by the parser), `""` when absent. Custom properties read case-sensitively.
fn native_get_property_value(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let name = normalize_property_name(&name);
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let value = style_map(&st, node_id)
        .get(name.as_str())
        .map(|v| v.as_ref().to_string())
        .unwrap_or_default();
    Ok(js_string!(value).into())
}

/// `style.removeProperty(name)` — drop the declaration and return its
/// previous value (`""` when absent).
fn native_remove_property(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let name = args
        .first()
        .map(|v| v.to_string(context).map(|s| s.to_std_string_lossy()))
        .transpose()?
        .unwrap_or_default();
    let name = normalize_property_name(&name);
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let mut st = rc.borrow_mut();
    let old = style_map(&st, node_id).get(name.as_str()).cloned();
    write_declaration(&mut st, node_id, &name, "");
    Ok(js_string!(old.as_ref().map(|v| v.as_ref()).unwrap_or_default()).into())
}
