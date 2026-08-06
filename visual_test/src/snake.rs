//! Snake game demo: the page runs the whole game in JS; Rust only supplies
//! the periodic `tick()` auto-refresh (every 150 ms) via `#[htui::commands]`.
//!
//! Direction is controlled with clickable buttons — there is no keyboard
//! event outside `<input>` fields, and an `<input>`'s value only becomes
//! readable by JS after commit (see `docs/examples-notes.md`).

use htui::events::EventBindings;

/// Bindings for the page: auto-invoke the JS `tick()` function every 150 ms.
#[htui::commands(auto = "tick", every_ms = 150)]
mod commands {}

/// Bindings for the page: the auto-refresh only.
pub fn bindings() -> EventBindings {
    let mut bindings = EventBindings::default();
    commands::register(&mut bindings);
    bindings
}
