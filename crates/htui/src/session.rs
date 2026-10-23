//! entry point for rendering a document.
//!
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

use crate::config::Config;
use crate::events::EventBindings;
use crate::resources::{ResourceError, ResourceRegistry};

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
        Default::default()
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
    /// scripts as JS globals. Takes
    /// effect on the next [`Session::render`].
    pub fn set_bindings(&mut self, bindings: EventBindings) {
        self.bindings = bindings;
    }

    /// The session's host-function bindings.
    pub fn bindings(&self) -> &EventBindings {
        &self.bindings
    }

    /// Render an HTML document in the terminal.
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
