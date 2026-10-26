//! htui example: terminal music player UI.
//!
//! Pure interface + interaction demo — no audio is played. Progress is
//! simulated with a `setInterval` tick; all state lives in the page's JS.
//! Run with `cargo run` from `example/` (Esc quits).

use htui::{Session, include_html};

fn main() {
    let page = include_html!("src/player.html");
    let mut session = Session::new();
    session.render_source(&page, "");
}
