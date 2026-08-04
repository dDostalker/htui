//! Host function bindings: Rust callbacks exposed as JS callables.
//!
//! The application registers host functions (named like `EventBindings.host`
//! entries) that JS scripts can invoke. Each host function receives a
//! [`HostCall`]: the JS arguments, the dispatch context (if any), a place to
//! push [`RenderCmd`]s and the Boa [`Context`] (so it can build return
//! values). Whatever the function returns becomes the JS call result.

use boa_engine::{Context, JsValue};

use crate::events::HtmlEvent;
use crate::tui::graph_api::RenderCmd;

/// Everything a host function needs to run a JS call.
pub struct HostCall<'a> {
    /// JS arguments passed by the caller.
    pub args: &'a [JsValue],
    /// The DOM event currently being dispatched, if the call happens inside a
    /// listener (synthetic calls like auto-refresh carry `None`).
    pub event: Option<&'a HtmlEvent>,
    /// Push render commands here (e.g. `RenderCmd::RenderHtml`) to request a
    /// re-render after the JS call returns.
    pub render: &'a mut Vec<RenderCmd>,
    /// The Boa engine context, available so host functions can construct
    /// complex return values (objects, arrays, …).
    pub context: &'a mut Context,
}

/// A Rust function callable from JavaScript.
pub type HostFn = Box<dyn FnMut(&mut HostCall) -> JsValue + Send>;
