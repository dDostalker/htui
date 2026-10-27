//! Tests for compile-time HTML / CSS / JS source loading used by hot reload.

use htui::{include_css, include_html, include_js};

#[test]
fn include_sources_embed_content_and_track_paths_in_debug() {
    let html = include_html!("tests/fixtures/hot_reload.html");
    let css = include_css!("tests/fixtures/hot_reload.css");
    let js = include_js!("tests/fixtures/hot_reload.js");

    assert!(html.as_str().contains("hot-reload-fixture"));
    assert!(css.as_str().contains("color: initial"));
    assert!(js.as_str().contains("__htuiJsFixture"));

    #[cfg(debug_assertions)]
    {
        assert!(
            html.path()
                .is_some_and(|path| path.ends_with("tests/fixtures/hot_reload.html"))
        );
        assert!(
            css.path()
                .is_some_and(|path| path.ends_with("tests/fixtures/hot_reload.css"))
        );
        assert!(
            js.path()
                .is_some_and(|path| path.ends_with("tests/fixtures/hot_reload.js"))
        );
    }

    #[cfg(not(debug_assertions))]
    {
        assert_eq!(html.path(), None);
        assert_eq!(css.path(), None);
        assert_eq!(js.path(), None);
    }
}
