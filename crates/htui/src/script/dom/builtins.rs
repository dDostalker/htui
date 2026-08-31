//! Global builtin installation: the `document` / `console` objects, `alert`
//! / `quit`, timers and `fetch`.

use boa_engine::{
    Context, JsNativeError, JsResult, NativeFunction, js_string,
    object::ObjectInitializer,
    property::{Attribute, PropertyDescriptorBuilder},
};

use super::{
    console::{console_error, console_log, console_warn, native_alert, native_quit},
    document::{
        native_create_comment, native_create_document_fragment, native_create_element,
        native_create_text_node, native_get_document_body, native_get_document_doctype,
        native_get_document_element, native_get_document_head, native_get_element_by_id,
    },
    fetch::native_fetch,
    js_fn, node_wrapper, with_state,
    timers::{
        native_clear_interval, native_clear_timeout, native_set_interval, native_set_timeout,
    },
    traverse::{native_query_selector, native_query_selector_all},
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
    Ok(())
}
