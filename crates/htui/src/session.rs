//! Render session — the single entry point for rendering a document.
//!
//! A [`Session`] owns the session-scoped `mem://` resource registry (see
//! [`crate::resources`]) and the render settings (see [`crate::config`]).
//! Everything a document can see — `<script src>`, `<link href>`, `<img
//! src>`, JS `fetch` — is scoped to the session that renders it, so parallel
//! sessions never share or collide resources.

use std::sync::Arc;

use crate::config::Config;
use crate::events::EventBindings;
use crate::resources::{ResourceError, ResourceRegistry};

/// One render session: a resource registry plus render settings.
///
/// ```
/// use htui::{Config, Session};
///
/// let mut config = Config::default();
/// config.zoom = 2.0;
///
/// let mut session = Session::new();
/// session.set_config(config);
/// session.register("mem://app.js", b"console.log('hi')".to_vec()).unwrap();
/// // session.render("<html>…</html>", "");
/// ```
#[derive(Debug)]
pub struct Session {
    registry: Arc<ResourceRegistry>,
    config: Config,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Create a session with an empty registry and the default config.
    pub fn new() -> Self {
        Self {
            registry: Arc::new(ResourceRegistry::new()),
            config: Config::default(),
        }
    }

    /// Register raw bytes under a virtual `mem://` path (see
    /// [`ResourceRegistry::register`]).
    pub fn register(&self, path: impl Into<String>, bytes: Vec<u8>) -> Result<(), ResourceError> {
        self.registry.register(path, bytes)
    }

    /// Remove a virtual entry (see [`ResourceRegistry::unregister`]).
    pub fn unregister(&self, path: &str) -> bool {
        self.registry.unregister(path)
    }

    /// Load the bytes registered under `path` (see [`ResourceRegistry::load`]).
    pub fn load(&self, path: &str) -> Result<Arc<Vec<u8>>, ResourceError> {
        self.registry.load(path)
    }

    /// Access the session's resource registry (for advanced uses such as
    /// manual pipeline driving with [`crate::resources::with_registry`]).
    pub fn registry(&self) -> &ResourceRegistry {
        &self.registry
    }

    /// Run `f` with the session's resources installed on this thread, for
    /// manual pipeline driving (e.g. headless previews that parse and draw
    /// outside [`Session::render`]). Equivalent to
    /// `resources::with_registry(self.registry(), f)`.
    pub fn with_registry<R>(&self, f: impl FnOnce() -> R) -> R {
        crate::resources::with_registry(Arc::clone(&self.registry), f)
    }

    /// The session's render settings.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Mutably access the session's render settings (zoom / theme / frame
    /// gap). Takes effect on the next [`Session::render`].
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Replace the session's render settings.
    pub fn set_config(&mut self, config: Config) {
        self.config = config;
    }

    /// Set the initial zoom factor (default 1.0). The user can still zoom
    /// interactively during a render; this only seeds the start value.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.config.zoom = zoom;
    }

    /// The session's initial zoom factor.
    pub fn zoom(&self) -> f32 {
        self.config.zoom
    }

    /// Render an HTML document in the terminal with keyboard navigation,
    /// mouse scrolling and zoom.
    ///
    /// This is the plain entry point without event handling; use
    /// [`Session::render_with_events`] to enable click / hover interaction.
    pub fn render(&self, html: &str, css: &str) {
        crate::tui::run::run(html, css, None, Arc::clone(&self.registry), self.config);
    }

    /// Render an HTML document with JS-like event handling enabled.
    ///
    /// In addition to everything [`Session::render`] does, elements that
    /// declare `onclick` / `onmouseover` / `onmouseout` attributes become hit
    /// regions, a dedicated hit-test thread records their positions, and
    /// events bubble through the DOM ancestor chain to the handlers
    /// registered in `bindings`. Hovering produces no visual highlight
    /// (matching a browser without CSS `:hover` rules) — only the
    /// `onmouseover` / `onmouseout` events fire. See the
    /// [`events`](crate::events) module for the event model.
    pub fn render_with_events(&self, html: &str, css: &str, bindings: EventBindings) {
        crate::tui::run::run(
            html,
            css,
            Some(bindings),
            Arc::clone(&self.registry),
            self.config,
        );
    }
}
