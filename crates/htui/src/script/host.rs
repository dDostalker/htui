//! Host function bindings: Rust callbacks exposed as JS callables.
//!
//! The application registers host functions (named like `EventBindings.host`
//! entries) that JS scripts can invoke. [`host_fn`] wraps a closure with a
//! fully Boa-free signature: the closure receives a [`HostCtx`] (the JS
//! arguments, the dispatch context, a place to push [`RenderCmd`]s) and
//! returns `Result<T, HostError>`. Whatever it returns becomes the JS call
//! result; `Err` throws a JS exception catchable with `try/catch`.
//!
//! [`HostCtx::arg`] uses the [`FromJs`] conversion layer and [`host_fn`]
//! return values the [`IntoJs`] layer, so hand-written bindings never touch
//! Boa value types. `#[htui::command]` builds on the same layer for the
//! common case.

use boa_engine::{Context, JsResult, JsValue};

use crate::events::HtmlEvent;
use crate::tui::graph_api::RenderCmd;

use super::convert::{FromJs, HostError, IntoJs, IntoJsError, JsObjectBuilder};

/// Boa-free handle passed to host-function closures.
///
/// All methods work with plain Rust types: arguments convert via [`FromJs`],
/// errors are [`HostError`] and return values go through [`IntoJs`]. The
/// underlying Boa [`Context`] is never exposed, so host functions written
/// against this type cannot leak engine types into application code.
pub struct HostCtx<'a> {
    args: &'a [JsValue],
    event: Option<&'a HtmlEvent>,
    render: &'a mut Vec<RenderCmd>,
    context: &'a mut Context,
}

impl<'a> HostCtx<'a> {
    /// Assemble a call handle from the engine's dispatch context.
    pub(crate) fn new(
        args: &'a [JsValue],
        event: Option<&'a HtmlEvent>,
        render: &'a mut Vec<RenderCmd>,
        context: &'a mut Context,
    ) -> Self {
        Self {
            args,
            event,
            render,
            context,
        }
    }

    /// Convert the i-th JS argument into `T`.
    ///
    /// Returns `Ok(None)` when the argument is missing, `undefined` or `null`.
    pub fn arg<T: FromJs>(&mut self, i: usize) -> Result<Option<T>, HostError> {
        let Some(v) = self.args.get(i) else {
            return Ok(None);
        };
        if v.is_undefined() || v.is_null() {
            return Ok(None);
        }
        T::from_js(v, self.context).map(Some).map_err(HostError::from)
    }

    /// Convert the i-th JS argument into `T`, erroring when it is absent.
    pub fn arg_required<T: FromJs>(&mut self, i: usize) -> Result<T, HostError> {
        self.arg(i)?
            .ok_or_else(|| HostError::type_error(format!("missing argument #{i}")))
    }

    /// The DOM event currently being dispatched, if the call happens inside a
    /// listener (synthetic calls like auto-refresh carry `None`).
    pub fn event(&self) -> Option<&'a HtmlEvent> {
        self.event
    }

    /// The render-command queue; push commands here to request a re-render
    /// after the JS call returns.
    pub fn render(&mut self) -> &mut Vec<RenderCmd> {
        self.render
    }

    /// Start building a JS object return value.
    pub fn object(&mut self) -> JsObjectBuilder {
        JsObjectBuilder::new()
    }

    /// Raw JS argument slice. Hidden: `#[htui::command]`-generated plumbing
    /// uses this to run `FromJs` conversions with browser `undefined`-fallback
    /// semantics.
    #[doc(hidden)]
    pub fn raw_args(&self) -> &'a [JsValue] {
        self.args
    }

    /// Mutable access to the Boa engine context. Hidden:
    /// `#[htui::command]`-generated plumbing uses this for `IntoJs`/`FromJs`
    /// conversion.
    #[doc(hidden)]
    pub fn context_mut(&mut self) -> &mut Context {
        &mut *self.context
    }
}

/// Wrap a closure as a [`HostFn`] with a Boa-free signature.
///
/// The closure receives a [`HostCtx`] and returns `Result<T, HostError>`:
/// arguments via [`HostCtx::arg`], return values as any [`IntoJs`] value (the
/// [`JsObjectBuilder`] from [`HostCtx::object`] implements it), errors via
/// [`HostError`]. No Boa type appears in the user code — closures can also
/// capture state (`Arc<Mutex<…>>`, …) for stateful commands.
///
/// ```ignore
/// bindings.command("describe", htui::script::host_fn(|ctx| {
///     let name: String = ctx.arg_required(0)?;
///     let mut obj = ctx.object();
///     obj.set("greeting", format!("hi {name}"));
///     Ok(obj)
/// }));
/// ```
pub fn host_fn<F, T>(mut f: F) -> HostFn
where
    F: for<'a> FnMut(&mut HostCtx<'a>) -> Result<T, HostError> + Send + 'static,
    T: IntoJs,
{
    Box::new(move |ctx: &mut HostCtx| match f(ctx) {
        Ok(v) => v.into_js(ctx.context_mut()),
        Err(e) => Err(e.into_js_error()),
    })
}

/// A Rust function callable from JavaScript.
///
/// Values are produced by [`host_fn`] or the `#[htui::command]` macro; don't
/// write one by hand. The closure receives the Boa-free [`HostCtx`] and the
/// `JsResult<JsValue>` in the signature is only ever built by the generated
/// plumbing, never by application code.
///
/// Returning `Err` throws a JS exception (catchable with `try/catch`).
pub type HostFn = Box<dyn FnMut(&mut HostCtx) -> JsResult<JsValue> + Send>;
