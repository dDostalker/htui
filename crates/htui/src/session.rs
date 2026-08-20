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
pub struct Session {
    registry: Arc<ResourceRegistry>,
    config: Config,
    /// Host-function bindings exposed to the document's scripts (used when
    /// [`Config::events`] is enabled).
    bindings: EventBindings,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Create a session with an empty registry, the default config (events
    /// enabled) and no host bindings.
    pub fn new() -> Self {
        Self {
            registry: Arc::new(ResourceRegistry::new()),
            config: Config::default(),
            bindings: EventBindings::default(),
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

    /// Set the session's host-function bindings, exposed to the document's
    /// scripts as JS globals (used when [`Config::events`] is enabled). Takes
    /// effect on the next [`Session::render`].
    pub fn set_bindings(&mut self, bindings: EventBindings) {
        self.bindings = bindings;
    }

    /// The session's host-function bindings.
    pub fn bindings(&self) -> &EventBindings {
        &self.bindings
    }

    /// Render an HTML document in the terminal.
    ///
    /// Whether JS-like event handling (click / hover / keyboard events, the
    /// script thread) is active is controlled by [`Config::events`] (default
    /// `true`). With events enabled the session's host bindings (see
    /// [`Session::set_bindings`]) are exposed to the document's scripts, hit
    /// regions are tracked, and events bubble through the DOM ancestor chain
    /// to the handlers registered in the bindings or in page JS. Hovering
    /// produces no visual highlight (matching a browser without CSS `:hover`
    /// rules) — only the `onmouseover` / `onmouseout` events fire. See the
    /// [`events`](crate::events) module for the event model.
    pub fn render(&mut self, html: &str, css: &str) {
        // The bindings are consumed by the script thread; an events-disabled
        // render never touches them.
        let bindings = if self.config.events {
            Some(std::mem::take(&mut self.bindings))
        } else {
            None
        };
        crate::tui::run::run(html, css, bindings, Arc::clone(&self.registry), self.config);
    }
}
