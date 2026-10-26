//! entry point for rendering a document.
//!
use crate::config::Config;
use crate::events::EventBindings;
use crate::resources::{ResourceError, ResourceRegistry};
/// ```rust,ignore
/// use htui::{Config, CssSource, HtmlSource, JsSource, Session};
///
/// let mut config = Config::default();
/// config.zoom = 2.0;
///
/// let mut session = Session::new();
/// session.set_config(config);
/// session.register("mem://app.js", b"console.log('hi')".to_vec()).unwrap();
/// let html = HtmlSource::inline("<html></html>");
/// let css = CssSource::inline("");
/// let js = JsSource::inline("console.log('ready')");
/// session.render(&html, &css, &js);
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

    /// Render HTML, session-level CSS, and initial session-level JS.
    ///
    /// This is the only rendering entry point: all three sources are required.
    /// Session JS runs after every `<script>` embedded in the HTML.
    ///
    /// Use [`include_html!`](crate::include_html),
    /// [`include_css!`](crate::include_css), and
    /// [`include_js!`](crate::include_js) to load the sources. Debug builds
    /// watch every source that has a path. Saving an HTML file reloads the
    /// whole document; saving CSS replaces session CSS; saving JS replaces and
    /// re-runs session JS without replacing the DOM. Release builds use only
    /// embedded content and contain no watcher code. Hot reload requires
    /// `Config.events` because HTML and JS must go through the script engine
    /// to keep the live DOM the single source of truth.
    pub fn render(
        &mut self,
        html: &crate::HtmlSource,
        css: &crate::CssSource,
        js: &crate::JsSource,
    ) {
        #[cfg(debug_assertions)]
        let paths = crate::tui::run::HotReloadPaths {
            html: html.path(),
            css: css.path(),
            js: js.path(),
        };

        #[cfg(debug_assertions)]
        if paths.any() && self.config.events {
            crate::tui::run::run_hot(paths, html.as_str(), css.as_str(), js.as_str(), self);
            return;
        }

        #[cfg(debug_assertions)]
        if paths.any() && !self.config.events {
            log::warn!("Hot reload requires Config.events; rendering statically");
        }

        crate::tui::run::run(html.as_str(), css.as_str(), js.as_str(), self);
    }
}
