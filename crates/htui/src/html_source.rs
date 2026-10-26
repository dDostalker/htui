//! Compile-time HTML source loading with optional debug hot-reload metadata.

/// An HTML document embedded at compile time.
///
/// [`include_html!`](crate::include_html) builds this type. In debug builds it
/// also carries the absolute source path so the renderer can watch that file;
/// in release builds the path is omitted and the value is purely static.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtmlSource {
    content: &'static str,
    path: Option<&'static str>,
}

impl HtmlSource {
    /// Create a source from compile-time content.
    ///
    /// `path` is normally supplied only by [`include_html!`](crate::include_html)
    /// in a debug build. The constructor is public because macro expansion
    /// occurs in dependent crates.
    pub const fn new(content: &'static str, path: Option<&'static str>) -> Self {
        Self { content, path }
    }

    /// The HTML embedded at compile time.
    pub const fn as_str(&self) -> &'static str {
        self.content
    }

    /// The absolute source path in a debug build, or `None` in release.
    pub const fn path(&self) -> Option<&str> {
        self.path
    }
}

/// Embed an HTML file and, in debug builds, remember its source path.
///
/// The path is relative to the current Cargo package root
/// (`CARGO_MANIFEST_DIR`), not to the invoking source file. This makes the
/// generated runtime path independent of the process working directory.
///
/// ```ignore
/// let page = htui::include_html!("src/player.html");
/// session.render_source(&page, "");
/// ```
///
/// The file is always embedded through `include_str!`, so a missing file is a
/// compile error and Cargo tracks edits to it. Only the debug expansion
/// includes the source path; release binaries contain neither the path nor the
/// hot-reload watcher.
#[macro_export]
macro_rules! include_html {
    ($path:literal $(,)?) => {{
        const __HTUI_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path));
        #[cfg(debug_assertions)]
        const __HTUI_HTML_PATH: Option<&str> =
            Some(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path));
        #[cfg(not(debug_assertions))]
        const __HTUI_HTML_PATH: Option<&str> = None;
        $crate::HtmlSource::new(__HTUI_HTML, __HTUI_HTML_PATH)
    }};
}
