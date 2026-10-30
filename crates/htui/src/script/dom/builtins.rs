//! Global builtin installation: the `document` / `console` objects, `alert`
//! / `quit`, timers, `queueMicrotask` and `fetch`.

use boa_engine::{
    Context, JsNativeError, JsResult, JsValue, NativeFunction,
    job::PromiseJob,
    js_string,
    object::ObjectInitializer,
    property::{Attribute, PropertyDescriptorBuilder},
};

use super::{
    console::{console_error, console_log, console_warn, native_alert, native_quit},
    document::{
        native_create_comment, native_create_document_fragment, native_create_element,
        native_create_text_node, native_get_character_set, native_get_compat_mode,
        native_get_content_type, native_get_document_body, native_get_document_doctype,
        native_get_document_element, native_get_document_head, native_get_document_title,
        native_get_document_uri, native_get_element_by_id, native_get_hidden,
        native_get_visibility_state, native_has_focus, native_set_document_title,
    },
    events::{custom_event_constructor, event_constructor},
    fetch::native_fetch,
    js_fn, node_wrapper,
    timers::{
        native_cancel_animation_frame, native_clear_interval, native_clear_timeout,
        native_request_animation_frame, native_set_interval, native_set_timeout,
    },
    traverse::{
        native_get_elements_by_class_name, native_get_elements_by_tag_name, native_query_selector,
        native_query_selector_all,
    },
    view::{
        native_add_window_event_listener, native_get_active_element, native_get_selection,
        native_inner_height, native_inner_width, native_match_media,
        native_remove_window_event_listener, native_scroll_by, native_scroll_to, native_scroll_y,
    },
    with_state,
};

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
        (
            "getElementsByTagName",
            NativeFunction::from_fn_ptr(native_get_elements_by_tag_name),
        ),
        (
            "getElementsByClassName",
            NativeFunction::from_fn_ptr(native_get_elements_by_class_name),
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
    // `title` is the one document member with a setter (rewrites the first
    // `<title>`'s text; the render thread picks it up on the next repaint).
    let title_desc = PropertyDescriptorBuilder::new()
        .get(js_fn(
            context,
            NativeFunction::from_fn_ptr(native_get_document_title),
        ))
        .set(js_fn(
            context,
            NativeFunction::from_fn_ptr(native_set_document_title),
        ))
        .enumerable(true)
        .configurable(true)
        .build();
    doc_obj.define_property_or_throw(js_string!("title"), title_desc, context)?;
    // Environment snapshots libraries sniff for (always-constant here).
    for (name, native) in [
        (
            "characterSet",
            NativeFunction::from_fn_ptr(native_get_character_set),
        ),
        (
            "charset",
            NativeFunction::from_fn_ptr(native_get_character_set),
        ),
        (
            "inputEncoding",
            NativeFunction::from_fn_ptr(native_get_character_set),
        ),
        (
            "contentType",
            NativeFunction::from_fn_ptr(native_get_content_type),
        ),
        (
            "compatMode",
            NativeFunction::from_fn_ptr(native_get_compat_mode),
        ),
        ("URL", NativeFunction::from_fn_ptr(native_get_document_uri)),
        (
            "documentURI",
            NativeFunction::from_fn_ptr(native_get_document_uri),
        ),
        (
            "visibilityState",
            NativeFunction::from_fn_ptr(native_get_visibility_state),
        ),
        ("hidden", NativeFunction::from_fn_ptr(native_get_hidden)),
    ] {
        let desc = PropertyDescriptorBuilder::new()
            .get(js_fn(context, native))
            .enumerable(true)
            .configurable(true)
            .build();
        doc_obj.define_property_or_throw(js_string!(name), desc, context)?;
    }
    doc_obj.set(
        js_string!("hasFocus"),
        js_fn(context, NativeFunction::from_fn_ptr(native_has_focus)),
        false,
        context,
    )?;
    doc_obj.set(
        js_string!("getSelection"),
        js_fn(context, NativeFunction::from_fn_ptr(native_get_selection)),
        false,
        context,
    )?;
    {
        let desc = PropertyDescriptorBuilder::new()
            .get(js_fn(
                context,
                NativeFunction::from_fn_ptr(native_get_active_element),
            ))
            .enumerable(true)
            .configurable(true)
            .build();
        doc_obj.define_property_or_throw(js_string!("activeElement"), desc, context)?;
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

    // queueMicrotask (runs on the script thread's microtask queue — the same
    // `run_jobs` pass that drains `.then` / `await` continuations)
    context.register_global_builtin_callable(
        js_string!("queueMicrotask"),
        1,
        NativeFunction::from_fn_ptr(native_queue_microtask),
    )?;

    // `window` — an alias of the global object (`window === globalThis`), so
    // the common `window.document` / `window.setTimeout` / `window.title`
    // spellings work unchanged. No window-specific members are added; there
    // is no separate window identity in htui.
    let global_obj = context.global_object();
    context.register_global_property(js_string!("window"), global_obj, Attribute::all())?;

    // Event / CustomEvent constructors (plain objects carrying the flags
    // and stop / prevent methods `dispatchEvent` reads). Built with the
    // constructor flag so `new Event(...)` works.
    let event_ctor = boa_engine::object::FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(event_constructor),
    )
    .name(js_string!("Event"))
    .length(1)
    .constructor(true)
    .build();
    context.register_global_property(js_string!("Event"), event_ctor, Attribute::all())?;
    let custom_ctor = boa_engine::object::FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(custom_event_constructor),
    )
    .name(js_string!("CustomEvent"))
    .length(1)
    .constructor(true)
    .build();
    context.register_global_property(js_string!("CustomEvent"), custom_ctor, Attribute::all())?;

    // Animation frames: browser-aligned one-shot callbacks on the session's
    // ~60fps frame grid (see `native_request_animation_frame`).
    context.register_global_builtin_callable(
        js_string!("requestAnimationFrame"),
        1,
        NativeFunction::from_fn_ptr(native_request_animation_frame),
    )?;
    context.register_global_builtin_callable(
        js_string!("cancelAnimationFrame"),
        1,
        NativeFunction::from_fn_ptr(native_cancel_animation_frame),
    )?;

    // Window view API: viewport / scroll getters and window-level events.
    // The getters read the shared UI state the input loop publishes (see
    // `SharedUiState`); `matchMedia` reuses the CSS `@media` grammar, so
    // JS and stylesheet queries agree by construction.
    for (name, native) in [
        (
            "innerWidth",
            NativeFunction::from_fn_ptr(native_inner_width),
        ),
        (
            "innerHeight",
            NativeFunction::from_fn_ptr(native_inner_height),
        ),
        ("scrollY", NativeFunction::from_fn_ptr(native_scroll_y)),
    ] {
        let desc = PropertyDescriptorBuilder::new()
            .get(js_fn(context, native))
            .enumerable(true)
            .configurable(true)
            .build();
        let global = context.global_object();
        global.define_property_or_throw(js_string!(name), desc, context)?;
    }
    context.register_global_builtin_callable(
        js_string!("getSelection"),
        0,
        NativeFunction::from_fn_ptr(native_get_selection),
    )?;
    for (name, native) in [
        (
            "matchMedia",
            NativeFunction::from_fn_ptr(native_match_media),
        ),
        ("scrollTo", NativeFunction::from_fn_ptr(native_scroll_to)),
        ("scrollBy", NativeFunction::from_fn_ptr(native_scroll_by)),
        (
            "addEventListener",
            NativeFunction::from_fn_ptr(native_add_window_event_listener),
        ),
        (
            "removeEventListener",
            NativeFunction::from_fn_ptr(native_remove_window_event_listener),
        ),
    ] {
        context.register_global_builtin_callable(js_string!(name), 1, native)?;
    }

    // XMLSerializer: `new XMLSerializer().serializeToString(node)` — the
    // node's outer-HTML form.
    let serializer_ctor = boa_engine::object::FunctionObjectBuilder::new(
        context.realm(),
        NativeFunction::from_fn_ptr(xml_serializer_constructor),
    )
    .name(js_string!("XMLSerializer"))
    .length(0)
    .constructor(true)
    .build();
    context.register_global_property(
        js_string!("XMLSerializer"),
        serializer_ctor,
        Attribute::all(),
    )?;
    Ok(())
}

/// `new XMLSerializer()` — installs `serializeToString` on the fresh
/// instance (calling without `new` still yields a usable object via the
/// implicit `this`).
fn xml_serializer_constructor(
    this: &JsValue,
    _args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let obj = this
        .to_object(context)
        .map_err(|_| JsNativeError::typ().with_message("XMLSerializer requires `new`"))?;
    let method = js_fn(
        context,
        NativeFunction::from_fn_ptr(xml_serializer_serialize),
    );
    let method_val: JsValue = method.into();
    obj.set(js_string!("serializeToString"), method_val, false, context)?;
    Ok(this.clone())
}

/// `XMLSerializer.serializeToString(node)` — the node's outer-HTML
/// serialization (element tags included; text / comment bodies).
fn xml_serializer_serialize(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let undefined = JsValue::undefined();
    let arg = args.first().unwrap_or(&undefined);
    let Some(obj) = arg.as_object() else {
        return Err(JsNativeError::typ()
            .with_message("serializeToString expects a Node")
            .into());
    };
    let id_val = obj.get(js_string!("_nodeId"), context)?;
    let Some(n) = id_val.as_number() else {
        return Err(JsNativeError::typ()
            .with_message("serializeToString expects a Node")
            .into());
    };
    let node_id = crate::script::dom::f64_to_node_id(n);
    let Some(rc) = with_state() else {
        return Ok(js_string!().into());
    };
    let st = rc.borrow();
    let html = crate::html::html_parse::serialize_outer_html(&st.arena, node_id);
    Ok(js_string!(html).into())
}

/// `queueMicrotask(callback)` — schedule `callback` on the engine's microtask
/// queue. The job runs when the current script / event dispatch finishes,
/// before the next timer or message (`ScriptEngine::run_jobs` drains it).
/// Errors thrown by the callback surface as job errors, logged by the event
/// loop like a failed `.then` handler.
pub(super) fn native_queue_microtask(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let Some(callback) = args.first().cloned() else {
        return Err(JsNativeError::typ()
            .with_message("queueMicrotask expects a function")
            .into());
    };
    if !callback.is_callable() {
        return Err(JsNativeError::typ()
            .with_message("queueMicrotask expects a function")
            .into());
    }
    // `PromiseJob` takes an `FnOnce` closure, so moving the callback in is
    // safe without `unsafe` (the `Copy` bound only applies to reusable
    // `NativeFunction` closures). Boa roots the captured function while the
    // job sits in the queue — the same mechanism promise reactions use.
    let callable = callback.as_object().expect("is_callable implies an object");
    context.enqueue_job(
        PromiseJob::new(move |ctx| callable.call(&JsValue::undefined(), &[], ctx)).into(),
    );
    Ok(JsValue::undefined())
}
