//! Snake game demo: the whole game runs in JS, including the game loop
//! (the page calls `setInterval(tick, 150)` itself). Rust registers no
//! commands — `bindings()` returns empty bindings.
//!
//! Direction is controlled with clickable buttons — there is no keyboard
//! event outside `<input>` fields, and an `<input>`'s value only becomes
//! readable by JS after commit (see `docs/examples-notes.md`).

use htui::events::EventBindings;

/// Bindings for the page: none (the game loop is a JS `setInterval`).
pub fn bindings() -> EventBindings {
    EventBindings::default()
}
