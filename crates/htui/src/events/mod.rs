//! JS-driven event system on top of the terminal HTML renderer.
//!
//! This module is a *side channel*: it does not participate in the
//! `html → tdom → layout → tui` rendering pipeline. Instead it records where
//! elements landed after a layout pass and lets the input loop translate
//! terminal mouse events into DOM-style events with bubbling.
//!
//! ## Architecture
//!
//! 1. The render thread publishes an immutable [`Geometry`] snapshot (DOM
//!    arena + laid-out boxes) after every redraw.
//! 2. A dedicated hit-test thread consumes the snapshot, walks the layout
//!    forest and builds a [`HitMap`] of [`HitRegion`]s — one rectangle per
//!    laid-out element, together with each element's DOM ancestor chain.
//! 3. The input loop hit-tests mouse coordinates against the [`HitMap`] and
//!    forwards the event to the [`ScriptEngine`](crate::script::ScriptEngine),
//!    which fires the JS listeners registered on the bubbling path
//!    (`addEventListener` or compiled `on*` attribute bodies).
//!
//! The pure interaction logic (enter / leave detection, click dispatch) lives
//! in [`interaction`] so it can be tested without a terminal.
//!
//! ## Event handler semantics (browser-aligned)
//!
//! Elements declare behaviour either with the familiar HTML event attributes
//!
//! ```html
//! <div onclick="open()" onmouseover="highlight()">…</div>
//! ```
//!
//! or programmatically from a `<script>` block:
//!
//! ```html
//! <script>
//!   document.getElementById('btn').addEventListener('click', function () { … });
//! </script>
//! ```
//!
//! Like a browser, an attribute value is *code*: it is compiled to
//! `function (event) { <body> }` and registered as a listener. A bare
//! identifier (`onclick="open"`) is evaluated but never invoked, matching the
//! browser's handler semantics. Scripts run in the Boa engine with the DOM
//! bindings described in [`crate::script`]; Rust application callbacks are
//! registered as **host functions** (see [`EventBindings::host`]) and become
//! JS globals callable from scripts.

use std::collections::HashMap;

use crate::{
    script::HostFn,
    tdom::node::NodeId,
};

mod geometry;
mod hit_map;
pub mod interaction;

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
    /// Fired when an element gains keyboard focus (`onfocus`). Only `<input>`
    /// fields are focusable today.
    Focus,
    /// Fired when an element loses keyboard focus (`onblur`).
    Blur,
    /// Fired whenever an `<input>` field's value changes while editing
    /// (`oninput`).
    Input,
    /// Fired when a field's value is committed (on blur / Enter) and differs
    /// from the value it had when it gained focus (`onchange`).
    Change,
    /// Fired when a key is pressed (`onkeydown`). Dispatched to the focused
    /// `<input>` (or the document `<body>`, falling back to the root, when
    /// none) and bubbles along the DOM ancestor chain. Page-level built-in
    /// shortcuts (scrolling / zoom / quit) are gated on this event: the input
    /// loop dispatches `keydown` to the script layer first and runs the
    /// built-in only when no listener called `preventDefault()` (see
    /// [`KeyGate`](crate::tui::graph_api::KeyGate)). Input-editing keys,
    /// `<select>` navigation and `Tab` focus cycling are input-control
    /// behaviour and are never gated.
    KeyDown,
    /// Fired when a key is released (`onkeyup`). Only emitted by terminals
    /// that report key releases (kitty keyboard protocol / Windows console
    /// mode); the dispatch target matches [`EventType::KeyDown`].
    KeyUp,
}

/// Terminal-agnostic keyboard payload carried by [`EventType::KeyDown`] /
/// [`EventType::KeyUp`], converted from a crossterm `KeyEvent` on the input
/// loop so the script layer stays independent of the terminal event source.
/// Field values mirror the browser `KeyboardEvent` shape (`key` / `code` /
/// `keyCode` + modifier flags), so JS listeners can branch on the same strings
/// they would in a real browser.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyEventInfo {
    /// `event.key` — the printed character (`"a"`, `" "`, `"A"`) or a named
    /// key (`"Enter"`, `"ArrowUp"`, `"F5"`, `"Escape"`, …).
    pub key: String,
    /// `event.code` — the physical key (`"KeyA"`, `"Space"`, `"ArrowUp"`,
    /// `"Enter"`, `"F5"`, …); independent of the active layout / modifiers.
    pub code: String,
    /// Browser-style numeric key code (`13` for Enter, `32` for Space,
    /// `112..123` for F1..F12, …). `0` when no DOM equivalent exists.
    pub key_code: u32,
    /// `event.shiftKey`.
    pub shift: bool,
    /// `event.ctrlKey`.
    pub ctrl: bool,
    /// `event.altKey`.
    pub alt: bool,
    /// `event.metaKey` (Super / Command / Windows key).
    pub meta: bool,
    /// `event.repeat` — whether this is an auto-repeated press.
    pub repeat: bool,
}

/// One element on a bubbling path (target first, then its ancestors).
///
/// `id` is the stable cross-session identity; `node_id` is the session-local
/// arena key used by the script engine to look up JS listeners.
#[derive(Debug, Clone)]
pub struct ElementRef {
    pub id: Option<String>,
    pub node_id: NodeId,
}

/// Event context delivered to a Rust host function, mirroring the DOM `Event`
/// shape. `target` is the deepest element under the pointer; `current_target`
/// is the element whose listener is being invoked (they differ when an
/// ancestor handles a bubbled event).
#[derive(Debug, Clone)]
pub struct HtmlEvent {
    /// The event being dispatched.
    pub event_type: EventType,
    /// `id` of the deepest element under the pointer.
    pub target: Option<String>,
    /// `id` of the element whose listener is currently running.
    pub current_target: Option<String>,
    /// Canvas coordinates (column / row in the virtual canvas, already offset
    /// by `scroll_y`).
    pub x: u32,
    pub y: u32,
    /// Always `true` — bubbling is the only supported propagation model.
    pub bubbles: bool,
    /// Keyboard payload for [`EventType::KeyDown`] / [`EventType::KeyUp`];
    /// `None` for pointer / form events.
    pub key: Option<KeyEventInfo>,
}

/// Application bindings for the script engine.
///
/// `host` maps JS global names to Rust callbacks (see [`HostFn`]). Every entry
/// is exposed to the document's scripts as a callable global, so JS code can
/// hand logic back to the application — e.g. `sample()` for a system monitor.
#[derive(Default)]
pub struct EventBindings {
    /// JS-callable host functions.
    pub host: HashMap<String, HostFn>,
}

impl EventBindings {
    /// Register one host function under a JS global name.
    ///
    /// The value is usually produced by `#[htui::command]` (which generates
    /// `<fn>_command()` returning a [`HostFn`]):
    ///
    /// ```ignore
    /// bindings.command("sample", sample_command());
    /// ```
    pub fn command(&mut self, name: impl Into<String>, host: HostFn) {
        self.host.insert(name.into(), host);
    }
}
