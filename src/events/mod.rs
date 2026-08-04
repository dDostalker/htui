//! JS-like event semantics on top of the terminal HTML renderer.
//!
//! This module is a *side channel*: it does not participate in the
//! `html → tdom → layout → tui` rendering pipeline. Instead it records where
//! interactive elements landed after a layout pass and lets the input loop
//! translate terminal mouse events into DOM-style events with bubbling.
//!
//! ## Architecture
//!
//! 1. The render thread publishes an immutable [`Geometry`] snapshot (DOM
//!    arena + laid-out boxes) after every `Redraw`.
//! 2. A dedicated hit-test thread consumes the snapshot, walks the layout
//!    forest and builds a [`HitMap`] of [`HitRegion`]s — one rectangle per
//!    element that declares an `onclick` / `onmouseover` / `onmouseout`
//!    attribute — together with each element's DOM ancestor chain.
//! 3. The input loop hit-tests mouse coordinates against the [`HitMap`] and
//!    dispatches [`EventType`]s through [`dispatch`], which walks the
//!    target's ancestor path (target → root) and evaluates each element's
//!    `on*` handler body.
//!
//! The pure interaction logic (enter / leave detection, click dispatch) lives
//! in [`interaction`] so it can be tested without a terminal. The handler
//! bodies themselves are evaluated by [`eval`] — a small JavaScript-subset
//! interpreter.
//!
//! ## Event handler semantics (browser-aligned)
//!
//! Elements declare behaviour with the familiar HTML event attributes:
//!
//! ```html
//! <div onclick="open()" onmouseover="highlight()">…</div>
//! ```
//!
//! Like a browser, the attribute value is *code*: it is parsed as a
//! JavaScript-subset expression and executed when the event fires. Function
//! names resolve against the [`EventBindings::actions`] registry — the
//! application's global scope — plus a few built-ins (`console.*`, `alert`,
//! `quit`). A bare identifier (`onclick="open"`) is evaluated but never
//! invoked, matching the browser's handler semantics. The event model
//! (`target`, `current_target`, `path`, `bubbles`) mirrors the DOM; a future
//! full JS engine can replace the subset evaluator behind the same interface.

use std::collections::HashMap;

use crate::tui::graph_api::RenderCmd;

mod dispatch;
pub mod eval;
mod geometry;
mod hit_map;
pub mod interaction;

pub use dispatch::dispatch;
pub use geometry::Geometry;
pub use hit_map::{HitMap, HitRegion};
pub use interaction::{HoverState, on_click, on_move};

/// DOM-like event types supported by the input layer.
///
/// Each variant maps 1:1 to the HTML event attribute of the same name
/// (`onclick` → [`EventType::Click`], …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    /// Fired when the left mouse button is pressed and released on the same
    /// cell of an interactive element (`onclick`).
    Click,
    /// Fired when the pointer enters an interactive element (`onmouseover`).
    MouseOver,
    /// Fired when the pointer leaves an interactive element (`onmouseout`).
    MouseOut,
}

/// One element on a bubbling path (target first, then its ancestors).
///
/// `id` is the stable cross-session identity delivered to handlers; `actions`
/// are the raw `on*` handler bodies (JavaScript-subset expressions) declared
/// by that element, consulted during bubbling dispatch.
#[derive(Debug, Clone)]
pub struct ElementRef {
    pub id: Option<String>,
    pub actions: HashMap<EventType, String>,
}

/// Event object delivered to a handler, mirroring the DOM `Event` shape.
///
/// `target` is the deepest element under the pointer; `current_target` is the
/// element whose handler is being invoked (they differ when an ancestor
/// handles a bubbled event). `action` carries the `on*` attribute value that
/// fired the event; `bubbles` is always `true` in the current implementation
/// (reserved so later work can model non-bubbling events).
#[derive(Debug)]
pub struct HtmlEvent<'a> {
    /// The event being dispatched.
    pub event_type: EventType,
    /// `id` of the deepest element under the pointer (the original target).
    pub target: Option<&'a str>,
    /// `id` of the element whose handler is being invoked (the bubbling
    /// current target). Differs from `target` when an ancestor handles the
    /// event.
    pub current_target: Option<&'a str>,
    /// Full bubbling chain: target first, then ancestors up to the root.
    pub path: &'a [ElementRef],
    /// The `on*` attribute value that fired the event.
    pub action: Option<&'a str>,
    /// Canvas coordinates (column / row in the virtual canvas, i.e. already
    /// offset by `scroll_y`).
    pub x: u32,
    pub y: u32,
    /// Always `true` — bubbling is the only supported propagation model.
    pub bubbles: bool,
}

/// User handler for one action name.
///
/// Returning `Some(RenderCmd)` lets a handler request a re-render (or exit)
/// after mutating application state; the command is forwarded to the render
/// thread by the input loop.
pub type EventHandler = Box<dyn FnMut(&HtmlEvent) -> Option<RenderCmd> + Send + 'static>;

/// Action registry consulted when an `on*` attribute fires.
///
/// Maps a **function name** — callable from an `onclick="buy()"` handler
/// body — to the handler that should run when that function is invoked. This
/// is the application's global scope for the event-handler evaluator.
#[derive(Default)]
pub struct EventBindings {
    pub actions: HashMap<String, EventHandler>,
}
