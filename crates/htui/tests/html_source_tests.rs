//! Tests for the compile-time HTML source loader used by debug hot reload.

use htui::include_html;

#[test]
fn include_html_embeds_content_and_tracks_the_source_in_debug() {
    let source = include_html!("tests/fixtures/hot_reload.html");

    assert!(
        source.as_str().contains("hot-reload-fixture"),
        "the fixture must be embedded at compile time"
    );

    #[cfg(debug_assertions)]
    {
        let path = source.path().expect("debug builds carry the source path");
        assert!(
            path.ends_with("tests/fixtures/hot_reload.html"),
            "debug path must point at the fixture: {path}"
        );
    }

    #[cfg(not(debug_assertions))]
    assert_eq!(source.path(), None, "release builds omit the source path");
}
