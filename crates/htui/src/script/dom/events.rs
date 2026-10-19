//! Event listener natives: `addEventListener` / `removeEventListener`, the
//! element focus requests and the JS-side listener registry helpers.

use boa_engine::{
    Context, JsNativeError, JsResult, JsValue, js_string, object::ObjectInitializer,
    property::Attribute,
};

use crate::{
    events::EventType,
    tdom::node::{NodeId, NodeType},
    tui::graph_api::RenderCmd,
};

use super::{node_id_to_f64, this_node_id, with_state};

pub(super) fn native_add_event_listener(
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

pub(super) fn native_remove_event_listener(
    this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let node_id = this_node_id(this, context)?;
    let event_type = parse_event_type(args.first(), context)?;
    remove_listener(context, node_id, event_type, args.get(1))?;
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
