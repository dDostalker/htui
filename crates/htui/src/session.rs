//! entry point for rendering a document.
//!
use crate::config::Config;
use crate::events::EventBindings;
use crate::resources::{ResourceError, ResourceRegistry};
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
use std::sync::Arc;

/// One render session is a resource registry plus render settings.
pub struct Session {
    registry: Arc<ResourceRegistry>,
    config: Config,
    bindings: EventBindings,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            registry: Arc::new(ResourceRegistry::new()),
            config: Config::default(),
            bindings: EventBindings::default(),
        }
    }

    /// Register raw bytes under a virtual `mem://` path
    pub fn register(&self, path: impl Into<String>, bytes: Vec<u8>) -> Result<(), ResourceError> {
        self.registry.register(path, bytes)
    }

    /// Remove a virtual entry
    pub fn unregister(&self, path: &str) -> bool {
        self.registry.unregister(path)
    }

    /// Load the bytes registered under `path`
    pub fn load(&self, path: &str) -> Result<Arc<Vec<u8>>, ResourceError> {
        self.registry.load(path)
    }

    /// Access the session's resource registry
    pub fn registry(&self) -> &ResourceRegistry {
        &self.registry
    }

    pub(crate) fn registry_handle(&self) -> Arc<ResourceRegistry> {
        Arc::clone(&self.registry)
    }

    pub(crate) fn take_bindings(&mut self) -> EventBindings {
        std::mem::take(&mut self.bindings)
    }

    /// Run `f` with the session's resources installed on this thread, for
    /// manual pipeline driving
    pub fn with_registry<R>(&self, f: impl FnOnce() -> R) -> R {
        crate::resources::with_registry(Arc::clone(&self.registry), f)
    }

    /// The session's render settings.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Mutably access the session's render settings
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Replace the session's render settings.
    pub fn set_config(&mut self, config: Config) {
        self.config = config;
    }

    /// Set the initial zoom factor . The user can still zoom
    /// interactively during a render before restart value.
    pub fn set_zoom(&mut self, zoom: f32) {
        self.config.zoom = zoom;
    }

    /// The session's initial zoom factor.
    pub fn zoom(&self) -> f32 {
        self.config.zoom
    }

    /// Set the session's host-function bindings, exposed to the document's
    /// scripts as JS globals. Takes effect on the next [`Session::render`].
    pub fn set_bindings(&mut self, bindings: EventBindings) {
        self.bindings = bindings;
    }

    /// Render an HTML document in the terminal.
    pub fn render(&mut self, html: &str, css: &str) {
        crate::tui::run::run(html, css, self);
    }

    /// Render a document loaded with [`include_html!`](crate::include_html).
    ///
    /// Debug builds start a file watcher when the source knows its path, so
    /// saving that HTML file reloads the whole document without restarting the
    /// process. Release builds use only the embedded content and contain no
    /// watcher code. Hot reload currently requires `Config.events`, because a
    /// whole-document replacement must go through the script engine to keep the
    /// live DOM the single source of truth.
    pub fn render_source(&mut self, source: &crate::HtmlSource, css: &str) {
        #[cfg(debug_assertions)]
        if let Some(path) = source.path()
            && self.config.events
        {
            crate::tui::run::run_hot(path, source.as_str(), css, self);
            return;
        }

        #[cfg(debug_assertions)]
        if source.path().is_some() && !self.config.events {
            log::warn!("Hot reload requires Config.events; rendering statically");
        }

        crate::tui::run::run(source.as_str(), css, self);
    }
}
