//! Virtual resource layer: an in-memory `mem://` registry.
//!
//! All resource-referencing features (`<img src>`, `<script src>`,
//! `<link href>`, and the JS `fetch` built-in) resolve through
//! [`ResourceRegistry::load`]. A document's assets live entirely in memory,
//! registered by the host via [`ResourceRegistry::register`] — no file
//! system access.
//!
//! The registry is **session-scoped**: the top-level [`Session`]
//! (crate::session::Session) owns one and installs it on the rendering and
//! script threads for the duration of a render (see [`with_registry`]).
//! There is no process-global registry — each render reads the session's
//! resources, so parallel documents never share or collide.
//!
//! Relative references are first resolved against the document base URL
//! (`<base href>`, see [`resolve_url`] and
//! [`crate::tdom::node::document_base_href`]), so a page under
//! `mem://app/` can use `src="img/pic.png"` and hit `mem://app/img/pic.png`.
//!
//! Paths are convenience-first: the `mem://` prefix is optional everywhere.
//! [`ResourceRegistry::register`], [`ResourceRegistry::load`] and
//! [`ResourceRegistry::unregister`] all normalise bare paths to `mem://…`
//! internally, so `register("app.js", …)` + `load("app.js")` (or
//! `load("mem://app.js")`) hit the same entry.

use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, Mutex},
};

/// Outcome of an asynchronous resource load, shipped back to the caller (the
/// script thread). `mem://` reads are instantaneous, but the fetch promise is
/// still settled through the engine's microtask queue.
#[derive(Debug)]
pub struct AsyncResult {
    /// The fetch id this result belongs to (routes back to the promise).
    pub id: u32,
    /// The URL / path the request was made for.
    pub url: String,
    /// The loaded bytes, or a description of the failure.
    pub outcome: Result<Arc<Vec<u8>>, String>,
}

/// Errors produced while resolving a resource.
#[derive(Debug, thiserror::Error)]
pub enum ResourceError {
    /// A registration used an empty path (the only way to register a useless
    /// key; `mem://` prefixes are auto-applied, so anything non-empty is
    /// valid).
    #[error("virtual resource path must not be empty")]
    EmptyPath,
    /// No entry was registered under the requested key.
    #[error("virtual resource not registered: {path}")]
    MemNotFound { path: String },
    /// A resource load ran on a thread without an installed registry.
    /// `Session::render` installs the session's registry on its rendering and
    /// script threads; a bare `Parser` / `TermLayout` driven manually outside
    /// that scope must wrap its work in [`with_registry`].
    #[error("no resource registry installed on this thread")]
    NoRegistry,
}

/// A `mem://` byte registry — one per [`Session`](crate::session::Session).
#[derive(Debug, Default)]
pub struct ResourceRegistry {
    map: Mutex<HashMap<String, Arc<Vec<u8>>>>,
}

impl ResourceRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register raw bytes under a virtual path. The `mem://` prefix is
    /// optional: `register("app.js", …)` and `register("mem://app.js", …)`
    /// are equivalent. Re-registering an existing key replaces its bytes.
    ///
    /// Fails only when `path` is empty (a bare `mem://` key is meaningless).
    pub fn register(&self, path: impl Into<String>, bytes: Vec<u8>) -> Result<(), ResourceError> {
        let path = path.into();
        if path.trim().is_empty() {
            return Err(ResourceError::EmptyPath);
        }
        let key = normalize(&path);
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, Arc::new(bytes));
        Ok(())
    }

    /// Remove a virtual entry. Returns `true` when it existed. The `mem://`
    /// prefix is optional.
    pub fn unregister(&self, path: &str) -> bool {
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&normalize(path))
            .is_some()
    }

    /// Load the bytes registered under `path`. The `mem://` prefix is
    /// optional.
    pub fn load(&self, path: &str) -> Result<Arc<Vec<u8>>, ResourceError> {
        let key = normalize(path);
        self.map
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .cloned()
            .ok_or(ResourceError::MemNotFound { path: key })
    }

    /// Deliver the bytes for `path` to `tx`, tagged with `id`. Always
    /// resolves synchronously (in-memory registry); the caller decides when
    /// to drain `tx`.
    pub fn load_async(&self, id: u32, path: String, tx: std::sync::mpsc::Sender<AsyncResult>) {
        let outcome = self.load(&path).map_err(|e| e.to_string());
        if tx
            .send(AsyncResult {
                id,
                url: path,
                outcome,
            })
            .is_err()
        {
            log::debug!("async resource load skipped: consumer already dropped");
        }
    }
}

thread_local! {
    /// The registry installed for the current thread's render / script
    /// work. `None` outside a [`with_registry`] scope (or a
    /// [`Session`](crate::session::Session) render).
    static CURRENT: RefCell<Option<Arc<ResourceRegistry>>> = const { RefCell::new(None) };
}

/// Run `f` with `registry` installed as the current thread's resource
/// registry. Every resource load in the scope — `<script src>`, `<link
/// href>`, `<img src>` decoding and JS `fetch` — resolves against it.
///
/// [`Session::render`](crate::session::Session::render) wraps its rendering
/// and script threads in this scope automatically; manual pipeline users
/// (driving [`crate::html::html_parse::Parser`] / [`TermLayout`]
/// (crate::tui::graph_api::TermLayout) directly) must wrap their work
/// themselves. Scopes nest; leaving restores the previous registry.
pub fn with_registry<R>(registry: Arc<ResourceRegistry>, f: impl FnOnce() -> R) -> R {
    CURRENT.with(|c| {
        let prev = c.borrow_mut().replace(registry);
        let result = f();
        if let Some(prev) = prev {
            c.borrow_mut().replace(prev);
        } else {
            c.borrow_mut().take();
        }
        result
    })
}

/// The registry installed for this thread, or `None` outside a
/// [`with_registry`] scope. Internal loads (`<script src>`, `<img>`, fetch)
/// go through this so the render / script threads never touch a global.
pub(crate) fn current() -> Option<Arc<ResourceRegistry>> {
    CURRENT.with(|c| c.borrow().clone())
}

/// Load a resource through the thread's installed registry (see
/// [`with_registry`]). Fails with [`ResourceError::NoRegistry`] when the
/// caller did not install one — sessions install theirs automatically.
pub(crate) fn load(path: &str) -> Result<Arc<Vec<u8>>, ResourceError> {
    match current() {
        Some(r) => r.load(path),
        None => Err(ResourceError::NoRegistry),
    }
}

/// Async variant of [`load`], used by the JS `fetch` bridge on the script
/// thread.
pub(crate) fn load_async(id: u32, path: String, tx: std::sync::mpsc::Sender<AsyncResult>) {
    match current() {
        Some(r) => r.load_async(id, path, tx),
        None => {
            let outcome = Err(ResourceError::NoRegistry.to_string());
            if tx
                .send(AsyncResult {
                    id,
                    url: path,
                    outcome,
                })
                .is_err()
            {
                log::debug!("async resource load skipped: consumer already dropped");
            }
        }
    }
}

/// Normalise a resource path to its canonical `mem://…` key. Bare paths gain
/// the prefix; already-prefixed paths pass through unchanged.
fn normalize(path: &str) -> String {
    if path.starts_with("mem://") {
        path.to_string()
    } else {
        format!("mem://{path}")
    }
}

/// Resolve a resource reference against an optional document base URL.
///
/// `<base href>` sets the document base URL
/// ([`crate::tdom::node::document_base_href`]); every relative reference
/// (`<script src>`, `<link href>`, `<img src>`, JS `fetch`) resolves against
/// it. Rules:
///
/// - Absolute references (any `scheme://` URL, including `mem://`) pass
///   through unchanged.
/// - Root-relative references (`/path`) resolve to `mem://path`.
/// - Relative references resolve against the base (`mem://dir/` + `x.png` →
///   `mem://dir/x.png`); a base without a trailing `/` is treated as a
///   directory.
/// - Without a base, the reference is a bare `mem://` key (prefixed when
///   needed), matching the pre-`<base>` behaviour.
pub fn resolve_url(base_href: Option<&str>, url: &str) -> String {
    // Already-absolute URLs (mem:// or any external scheme) pass through.
    if url.contains("://") {
        return url.to_string();
    }
    let Some(base) = base_href.filter(|b| !b.trim().is_empty()) else {
        return normalize(url);
    };
    if url.starts_with('/') {
        return format!("mem://{}", url.trim_start_matches('/'));
    }
    let base = normalize(base);
    format!("{}/{url}", base.trim_end_matches('/'))
}
