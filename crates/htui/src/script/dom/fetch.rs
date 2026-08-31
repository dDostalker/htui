//! `fetch` natives: async resource loads through the unified resource layer.

use boa_engine::{
    Context, JsNativeError, JsResult, JsString, JsValue, NativeFunction, js_string,
    object::{ObjectInitializer, builtins::JsPromise},
    property::Attribute,
};

use crate::{resources, tdom::node::document_base_href};

use super::{js_fn, with_state};

/// JS `fetch(url)` — resolves through the unified resource layer.
///
/// Returns a `Promise` that fulfils with a `Response`-like object (`status`,
/// `ok`, `text()`). Completion is reported back to
/// [`ScriptEngine::poll_fetches`] (crate::script::ScriptEngine::poll_fetches),
/// which settles the promise and runs the microtask queue.
pub(super) fn native_fetch(
    _this: &JsValue,
    args: &[JsValue],
    context: &mut Context,
) -> JsResult<JsValue> {
    let url = args
        .first()
        .map(|v| v.to_string(context))
        .transpose()?
        .unwrap_or_default();
    let url = url.to_std_string_escaped();
    if url.is_empty() {
        return Err(JsNativeError::typ()
            .with_message("fetch expects a non-empty URL")
            .into());
    }
    let Some(rc) = with_state() else {
        return Ok(JsValue::undefined());
    };

    // Create a pending promise. Its resolvers are GC-rooted in the
    // `__htuiFetchResolvers` JS registry (keyed by fetch id); keeping them in
    // Rust would leave them invisible to Boa's collector and they could be
    // reclaimed before the load completes.
    let (promise, resolvers) = JsPromise::new_pending(context);
    let (id, tx) = {
        let mut st = rc.borrow_mut();
        let id = st.next_fetch_id;
        st.next_fetch_id += 1;
        st.active_fetches.insert(id);
        (id, st.fetch_tx.clone())
    };
    let global = context.global_object();
    let registry = global
        .get(js_string!("__htuiFetchResolvers"), context)?
        .to_object(context)?;
    let mut entry = ObjectInitializer::new(context);
    entry.property(
        js_string!("resolve"),
        JsValue::from(resolvers.resolve.clone()),
        Attribute::all(),
    );
    entry.property(
        js_string!("reject"),
        JsValue::from(resolvers.reject.clone()),
        Attribute::all(),
    );
    registry.set(js_string!(id.to_string()), entry.build(), false, context)?;

    // Kick off the load. The result is tagged with the fetch id
    // so `poll_fetches` can route it back to the right promise. Relative
    // URLs resolve against the document base URL (`<base href>`) the same
    // way `<img src>` / `<script src>` do.
    let resolved = {
        let st = rc.borrow();
        let base = document_base_href(&st.arena, st.root_id);
        resources::resolve_url(base.as_deref(), &url)
    };
    resources::load_async(id, resolved, tx);
    Ok(promise.into())
}

/// Build a `Response`-like object for a fulfilled fetch: `status` (200), `ok`
/// (true), `url`, and a `text()` method returning a promise of the body as
/// UTF-8 text. The body bytes are captured in the closure, so `text()` can be
/// called repeatedly.
pub(crate) fn response_object(
    context: &mut Context,
    url: String,
    bytes: std::sync::Arc<Vec<u8>>,
) -> JsResult<JsValue> {
    let text = js_string!(String::from_utf8_lossy(bytes.as_ref()).into_owned());
    let text_native =
        |_this: &JsValue, _args: &[JsValue], captured: &JsString, ctx: &mut Context| {
            let (promise, resolvers) = JsPromise::new_pending(ctx);
            resolvers.resolve.call(
                &JsValue::undefined(),
                &[JsValue::from(captured.clone())],
                ctx,
            )?;
            Ok(promise.into())
        };
    let text_fn = js_fn(
        context,
        NativeFunction::from_copy_closure_with_captures(text_native, text),
    );
    let mut init = ObjectInitializer::new(context);
    init.property(js_string!("status"), JsValue::from(200), Attribute::all());
    init.property(js_string!("ok"), JsValue::from(true), Attribute::all());
    init.property(
        js_string!("url"),
        JsValue::from(js_string!(url)),
        Attribute::all(),
    );
    init.property(js_string!("text"), text_fn, Attribute::all());
    Ok(init.build().into())
}
