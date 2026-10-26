//! JavaScript scripting layer over the Boa engine.
//!
//! # Architecture
//!
//! A [`ScriptEngine`] owns a Boa [`Context`](crate::boa_engine::Context) plus a mutable DOM
//! ([`NodeArena`](crate::tdom::node::NodeArena)) that scripts manipulate
//! through browser-aligned bindings
//! (`document.getElementById`, `el.textContent`, `el.addEventListener`, …).
//!
//! The engine is a *side channel* like the rest of the event system: the
//! render thread still owns layout/paint, but when JS mutates the DOM the
//! engine reports a dirty flag and the input loop forwards the live arena to
//! the render thread via [`RenderCmd::RenderArena`](crate::tui::graph_api::RenderCmd::RenderArena)
//! — no HTML re-parse needed.
//!
//! # Event model
//!
//! `on*` attribute bodies are compiled to `function (event) { body }` (browser
//! semantics: a bare identifier only evaluates, it never calls). Each handler
//! lives in a single slot per (element, event type) — installing overwrites,
//! `removeAttribute` clears — and is (re)wired whenever its attribute appears:
//! at engine startup, after `innerHTML` writes, and after mutations
//! (`appendChild` / `insertBefore` / `replaceChild` / `cloneNode`) and
//! `setAttribute`. `addEventListener(type, fn)` registers JS functions on a
//! separate multicast list (GC-safe); at dispatch the attribute handler runs
//! after that element's list listeners. Dispatch walks the hit region's
//! ancestor path target-first, firing each element's handlers with a
//! DOM-style `Event` object (`target`, `currentTarget`, `x`, `y`,
//! `stopPropagation()`).
//!
//! # Host functions
//!
//! Entries of [`EventBindings::host`](crate::events::EventBindings::host)
//! become JS globals callable from scripts;
//! they can push [`RenderCmd`](crate::tui::graph_api::RenderCmd)s and return
//! values. See `host`.

mod bindings;
mod dom;
mod engine;
mod event_loop;
mod host;
mod msg;
mod state;

pub mod convert;

pub use convert::{FromJs, HostError, IntoJs, JsObjectBuilder};
pub use engine::ScriptEngine;
pub use host::{HostCtx, HostFn, host_fn};
pub use msg::{
    ScriptDirty, ScriptEvent, ScriptMsg, ScriptResponse, apply_message, run_script_thread,
    run_script_thread_with_js,
};
pub(crate) use state::{ENGINE_STATE, ScriptState, TimerEntry};

use state::{EngineBuildError, EngineParts};
