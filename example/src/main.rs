//! htui example: terminal music player UI.
//!
//! Pure interface + interaction demo — no audio is played. Progress is
//! simulated with a `setInterval` tick; all state lives in the page's JS.
//! Run with `cargo run` from `example/` (Esc quits).

use htui::{Session, include_css, include_html, include_js};

fn main() {
    let page = include_html!("src/player.html");
    let style = include_css!("src/player.css");
    let init = include_js!("src/player.init.js");
    let mut session = Session::new();
    session.render(&page, &style, &init);
}
