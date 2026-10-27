//! Compile-time HTML / CSS / JS source loading with debug hot-reload metadata.
//!
//! Each source type is a distinct newtype so `Session::render` cannot mix up its
//! three inputs. All wrappers share the same small runtime representation.

/// The language carried by a [`Source`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Html,
    Css,
    Js,
}

/// Language-agnostic compile-time source text and optional debug path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    kind: SourceKind,
    content: &'static str,
    path: Option<&'static str>,
}

impl Source {
    /// Create a source from compile-time content.
    pub const fn new(kind: SourceKind, content: &'static str, path: Option<&'static str>) -> Self {
        Self {
            kind,
            content,
            path,
        }
    }

    /// The embedded content.
    pub const fn as_str(&self) -> &'static str {
        self.content
    }

    /// The absolute source path in a debug build, or `None` in release.
    pub const fn path(&self) -> Option<&str> {
        self.path
    }

    /// Which render input this source represents.
    pub const fn kind(&self) -> SourceKind {
        self.kind
    }
}

macro_rules! define_source {
    ($name:ident, $kind:expr) => {
        /// A compile-time source loaded by the matching `include_*!` macro.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name(Source);

        impl $name {
            /// Create a source. `path` is supplied by debug macro expansion.
            pub const fn new(content: &'static str, path: Option<&'static str>) -> Self {
                Self(Source::new($kind, content, path))
            }

            /// Create source text without a hot-reload path.
            pub const fn inline(content: &'static str) -> Self {
                Self::new(content, None)
            }

            /// The embedded content.
            pub const fn as_str(&self) -> &'static str {
                self.0.as_str()
            }

            /// The absolute source path in a debug build, or `None` in release.
            pub const fn path(&self) -> Option<&str> {
                self.0.path()
            }

            /// Which render input this source represents.
            pub const fn kind(&self) -> SourceKind {
                self.0.kind()
            }
        }

        impl From<$name> for Source {
            fn from(value: $name) -> Self {
                value.0
            }
        }
    };
}

define_source!(HtmlSource, SourceKind::Html);
define_source!(CssSource, SourceKind::Css);
define_source!(JsSource, SourceKind::Js);

/// Shared expansion for all `include_*!` macros. In debug builds it embeds the
/// absolute source path; in release builds the path constant is removed.
#[doc(hidden)]
#[macro_export]
macro_rules! include_source {
    ($source:ident, $path:literal $(,)?) => {{
        const __HTUI_SOURCE: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path));
        #[cfg(debug_assertions)]
        const __HTUI_SOURCE_PATH: Option<&str> =
            Some(concat!(env!("CARGO_MANIFEST_DIR"), "/", $path));
        #[cfg(not(debug_assertions))]
        const __HTUI_SOURCE_PATH: Option<&str> = None;
        $crate::$source::new(__HTUI_SOURCE, __HTUI_SOURCE_PATH)
    }};
}

/// Embed an HTML file and, in debug builds, remember its source path.
///
/// The path is relative to the current Cargo package root
/// (`CARGO_MANIFEST_DIR`), not the invoking source file. The file is still
/// embedded with `include_str!`, so missing files fail at compile time and
/// Cargo tracks edits.
#[macro_export]
macro_rules! include_html {
    ($path:literal $(,)?) => {
        $crate::include_source!(HtmlSource, $path)
    };
}

/// Embed a CSS file and, in debug builds, remember its source path.
///
/// See [`include_html!`](crate::include_html) for path and release semantics.
#[macro_export]
macro_rules! include_css {
    ($path:literal $(,)?) => {
        $crate::include_source!(CssSource, $path)
    };
}

/// Embed a JS file and, in debug builds, remember its source path.
///
/// See [`include_html!`](crate::include_html) for path and release semantics.
#[macro_export]
macro_rules! include_js {
    ($path:literal $(,)?) => {
        $crate::include_source!(JsSource, $path)
    };
}
