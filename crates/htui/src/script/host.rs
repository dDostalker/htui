//! Host function bindings: Rust callbacks exposed as JS callables.
//!
//! The application registers host functions (named like `EventBindings.host`
//! entries) that JS scripts can invoke. Each host function receives a
//! [`HostCall`]: the JS arguments, the dispatch context (if any), a place to
//! push [`RenderCmd`]s and the Boa [`Context`] (so it can build return
//! values). Whatever the function returns becomes the JS call result.
//!
//! [`HostCall::arg`] / [`HostCall::ret`] use the [`IntoJs`] / [`FromJs`]
//! conversion layer so hand-written bindings never touch Boa value types; the
//! `#[htui::command]` macro builds on the same layer for the common case.

use boa_engine::{Context, JsError, JsResult, JsValue};

use crate::events::HtmlEvent;
use crate::tui::graph_api::RenderCmd;

use super::convert::{FromJs, IntoJs};

/// Everything a host function needs to run a JS call.
pub struct HostCall<'a> {
    /// JS arguments passed by the caller.
    pub args: &'a [JsValue],
    /// The DOM event currently being dispatched, if the call happens inside a
    /// listener (synthetic calls like auto-refresh carry `None`).
    pub event: Option<&'a HtmlEvent>,
    /// Push render commands here to request a re-render after the JS call
    /// returns. `RenderCmd::RenderHtml` is intercepted in-engine
    /// (`ScriptEngine::reload`), keeping the engine the document's source of
    /// truth.
    pub render: &'a mut Vec<RenderCmd>,
    /// The Boa engine context, available so host functions can construct
    /// complex return values (objects, arrays, …).
    pub context: &'a mut Context,
}

impl<'a> HostCall<'a> {
    /// Convert the i-th JS argument into `T`.
    ///
    /// Returns `Ok(None)` when the argument is missing, `undefined` or `null`.
    pub fn arg<T: FromJs>(&mut self, i: usize) -> Result<Option<T>, JsError> {
        let Some(v) = self.args.get(i) else {
            return Ok(None);
        };
        if v.is_undefined() || v.is_null() {
            return Ok(None);
        }
        T::from_js(v, self.context).map(Some)
    }

    /// Convert the i-th JS argument into `T`, erroring when it is absent.
    pub fn arg_required<T: FromJs>(&mut self, i: usize) -> Result<T, JsError> {
        self.arg(i)?.ok_or_else(|| {
            JsError::from_native(boa_engine::JsNativeError::typ()
                .with_message(format!("missing argument #{i}")))
        })
    }

    /// Convert a Rust value into the JS call result.
    ///
    /// Conversion errors are logged and fall back to `undefined`.
    pub fn ret<T: IntoJs>(&mut self, v: T) -> JsValue {
        match v.into_js(self.context) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Host return conversion failed: {e}");
                JsValue::undefined()
            }
        }
    }
}

/// A Rust function callable from JavaScript.
///
/// Returning `Err` throws a JS exception (catchable with `try/catch`).
pub type HostFn = Box<dyn FnMut(&mut HostCall) -> JsResult<JsValue> + Send>;
