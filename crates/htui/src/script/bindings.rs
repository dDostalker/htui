use std::cell::RefCell;
use std::rc::Rc;

use boa_engine::{Context, JsResult, JsValue, NativeFunction, Source, js_string};

use crate::{events::EventType, tdom::node::NodeId};

use super::dom::{clear_node_attr_handler, clear_node_listeners, set_attr_handler, with_state};
use super::host::HostCtx;
use super::state::ScriptState;

/// `event.stopPropagation()` — marks the event as stopped.
pub(super) fn stop_propagation(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Ok(obj) = this.to_object(context)
        && let Err(e) = obj.set(js_string!("_stopped"), true, false, context)
    {
        log::warn!("Failed to mark event stopped: {e}");
    }
    Ok(JsValue::undefined())
}

/// `event.preventDefault()` — marks the event so the input loop can skip the
/// built-in shortcut for that key (the verdict is echoed back in the script
/// thread's response; the input loop waits for it before acting).
pub(super) fn prevent_default(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    if let Ok(obj) = this.to_object(context) {
        for flag in ["_prevented", "defaultPrevented"] {
            if let Err(e) = obj.set(js_string!(flag), true, false, context) {
                log::warn!("Failed to mark event prevented: {e}");
                break;
            }
        }
    }
    Ok(JsValue::undefined())
}

/// Register one host function from `EventBindings.host` as a JS global.
pub(super) fn register_host_global(context: &mut Context, name: &str) -> JsResult<()> {
    let name_capture = name.to_owned();
    let native = NativeFunction::from_copy_closure_with_captures(
        |_this, args, name: &String, context| dispatch_host(name, args, context),
        name_capture,
    );
    context.register_global_builtin_callable(js_string!(name), 0, native)
}

/// Native dispatcher for host functions: looks the function up in the shared
/// state and calls it with a [`HostCtx`].
fn dispatch_host(name: &str, args: &[JsValue], context: &mut Context) -> JsResult<JsValue> {
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };
    let mut st = rc.borrow_mut();
    let ScriptState {
        host_fns,
        current_event,
        render_cmds,
        ..
    } = &mut *st;
    let Some(f) = host_fns.get_mut(name) else {
        log::warn!("Unknown host function `{name}`");
        return Ok(JsValue::undefined());
    };
    let mut call = HostCtx::new(args, current_event.as_ref(), render_cmds, context);
    f(&mut call)
}

/// The `on*` content attributes the engine wires to DOM events, mapped to
/// their [`EventType`]. Shared by the boot-time walk, the mutation-time
/// subtree compilation, and the `setAttribute` / `removeAttribute` hooks.
pub(super) const ATTRIBUTE_EVENT_TYPES: &[(&str, EventType)] = &[
    ("onclick", EventType::Click),
    ("onmouseover", EventType::MouseOver),
    ("onmouseout", EventType::MouseOut),
    ("onfocus", EventType::Focus),
    ("onblur", EventType::Blur),
    ("oninput", EventType::Input),
    ("onchange", EventType::Change),
    ("onkeydown", EventType::KeyDown),
    ("onkeyup", EventType::KeyUp),
    ("onsubmit", EventType::Submit),
    ("onreset", EventType::Reset),
    ("ontoggle", EventType::Toggle),
    ("onclose", EventType::Close),
];

/// Map an `on*` attribute name to the event type it wires; `None` for any
/// other attribute.
pub(super) fn attribute_event_type(name: &str) -> Option<EventType> {
    ATTRIBUTE_EVENT_TYPES
        .iter()
        .find(|(attr, _)| *attr == name)
        .map(|(_, et)| *et)
}

/// Walk the arena and compile every `on*` attribute body into a listener,
/// exactly like a browser does.
pub(super) fn compile_attribute_listeners(
    context: &mut Context,
    state: &Rc<RefCell<ScriptState>>,
) -> JsResult<()> {
    let node_ids: Vec<NodeId> = state.borrow().arena.iter().map(|(id, _)| id).collect();
    for node_id in node_ids {
        compile_node_attribute_listeners(context, state, node_id)?;
    }
    Ok(())
}

/// Compile `on*` attributes for a freshly inserted subtree (dynamic DOM:
/// `innerHTML` writes, `appendChild` / `insertBefore` / `replaceChild` /
/// `cloneNode`). Browser semantics: an `on*` attribute wires its handler
/// whenever the attribute is present, no matter when the node entered the
/// document. Handler slots are single (overwrite), so re-compiling a moved
/// node is idempotent.
pub(super) fn compile_attribute_listeners_for_subtree(
    context: &mut Context,
    state: &Rc<RefCell<ScriptState>>,
    root_id: NodeId,
) -> JsResult<()> {
    let node_ids = {
        let st = state.borrow();
        let mut out = Vec::new();
        let mut stack = vec![root_id];
        while let Some(id) = stack.pop() {
            out.push(id);
            if let Some(node) = st.arena.get(id) {
                stack.extend(node.children.iter().copied());
            }
        }
        out
    };
    for node_id in node_ids {
        compile_node_attribute_listeners(context, state, node_id)?;
    }
    Ok(())
}

pub(super) fn clear_attribute_handlers_for_subtree(
    context: &mut Context,
    state: &Rc<RefCell<ScriptState>>,
    root_id: NodeId,
) -> JsResult<()> {
    let node_ids = {
        let st = state.borrow();
        let mut out = Vec::new();
        let mut stack = vec![root_id];
        while let Some(id) = stack.pop() {
            out.push(id);
            if let Some(node) = st.arena.get(id) {
                stack.extend(node.children.iter().copied());
            }
        }
        out
    };
    for node_id in node_ids {
        clear_node_attribute_listeners(context, node_id)?;
    }
    Ok(())
}

/// Clear every attribute-handler slot of one node (all event types).
pub(super) fn clear_node_attribute_listeners(
    context: &mut Context,
    node_id: NodeId,
) -> JsResult<()> {
    clear_node_attr_handler(context, node_id)
}

/// Drop every multicast `addEventListener` listener of a subtree (
pub(super) fn clear_listeners_for_subtree(
    context: &mut Context,
    state: &Rc<RefCell<ScriptState>>,
    root_id: NodeId,
) -> JsResult<()> {
    let node_ids = {
        let st = state.borrow();
        let mut out = Vec::new();
        let mut stack = vec![root_id];
        while let Some(id) = stack.pop() {
            out.push(id);
            if let Some(node) = st.arena.get(id) {
                stack.extend(node.children.iter().copied());
            }
        }
        out
    };
    for node_id in node_ids {
        clear_node_listeners(context, node_id)?;
    }
    Ok(())
}

/// Compile every `on*` attribute of one node into the single-slot attribute
/// handler registry. Overwrites previous slots (browser semantics: the
/// latest attribute value wins), so re-running after `setAttribute` is
/// idempotent and can never double-fire.
pub(super) fn compile_node_attribute_listeners(
    context: &mut Context,
    state: &Rc<RefCell<ScriptState>>,
    node_id: NodeId,
) -> JsResult<()> {
    let attr_bodies = {
        let st = state.borrow();
        st.arena.get(node_id).map(|n| {
            ATTRIBUTE_EVENT_TYPES
                .iter()
                .filter_map(|(attr, et)| n.attributes.get(*attr).cloned().map(|b| (b, *et)))
                .collect::<Vec<_>>()
        })
    };
    let Some(attr_bodies) = attr_bodies else {
        return Ok(());
    };

    for (body, event_type) in attr_bodies {
        // `(function (event) { <body> })` — attribute value is *code*.
        let wrapped = format!("(function (event) {{ {body} }})");
        let fn_val = match context.eval(Source::from_bytes(wrapped.as_bytes())) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to compile attribute handler `{body}`: {e}");
                continue;
            }
        };
        if let Err(e) = set_attr_handler(context, node_id, event_type, &fn_val) {
            log::warn!(
                "Failed to register attribute handler for {:?}: {e}",
                node_id
            );
        }
    }
    Ok(())
}
