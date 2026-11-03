//! htui example: native rendering of the saved shell.html stress-test page.
//!
//! The page is self-contained (CSS inlined and images encoded as data URLs);
//! scripts and iframes were removed per htui's documented exclusions. Run
//! with `cargo run` from `example/` (Esc quits).

use htui::{Session, include_css, include_html, include_js};

fn main() {
    let page = include_html!("src/shell.html");
    let style = include_css!("src/shell.session.css");
    let init = include_js!("src/shell.init.js");
    let mut session = Session::new();
    session.render(&page, &style, &init);
}
