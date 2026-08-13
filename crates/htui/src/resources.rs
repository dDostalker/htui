//! Virtual resource layer: an in-memory `mem://` registry.
//!
//! All resource-referencing features (`<img src>`, `<script src>`,
//! `<link href>`, and the JS `fetch` built-in) resolve through [`load`]. A
//! document's assets live entirely in memory, registered by the host via
//! [`register`] — no file system access.
//!
//! Relative references are first resolved against the document base URL
//! (`<base href>`, see [`resolve_url`] and
//! [`crate::tdom::node::document_base_href`]), so a page under
//! `mem://app/` can use `src="img/pic.png"` and hit `mem://app/img/pic.png`.
//!
//! Paths are convenience-first: the `mem://` prefix is optional everywhere.
//! [`register`], [`load`] and [`unregister`] all normalise bare paths to
//! `mem://…` internally, so `register("app.js", …)` + `load("app.js")` (or
//! `load("mem://app.js")`) hit the same entry.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
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

/// Deliver the bytes for `path` to `tx`, tagged with `id`. Always resolves
/// synchronously (in-memory registry); the caller decides when to drain `tx`.
pub fn load_async(id: u32, path: String, tx: std::sync::mpsc::Sender<AsyncResult>) {
    let outcome = load(&path).map_err(|e| e.to_string());
    if tx.send(AsyncResult { id, url: path, outcome }).is_err() {
        log::debug!("async resource load skipped: consumer already dropped");
    }
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
}

/// Process-wide `mem://` byte registry.
static MEM: OnceLock<Mutex<HashMap<String, Arc<Vec<u8>>>>> = OnceLock::new();

fn mem_map() -> &'static Mutex<HashMap<String, Arc<Vec<u8>>>> {
    MEM.get_or_init(|| Mutex::new(HashMap::new()))
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

/// Register raw bytes under a virtual path. The `mem://` prefix is optional:
/// `register("app.js", …)` and `register("mem://app.js", …)` are equivalent.
/// Re-registering an existing key replaces its bytes.
///
/// Fails only when `path` is empty (a bare `mem://` key is meaningless).
pub fn register(path: impl Into<String>, bytes: Vec<u8>) -> Result<(), ResourceError> {
    let path = path.into();
    if path.trim().is_empty() {
        return Err(ResourceError::EmptyPath);
    }
    let key = normalize(&path);
    mem_map()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, Arc::new(bytes));
    Ok(())
}

/// Remove a virtual entry. Returns `true` when it existed. The `mem://`
/// prefix is optional.
pub fn unregister(path: &str) -> bool {
    mem_map()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&normalize(path))
        .is_some()
}

/// Load the bytes registered under `path`. The `mem://` prefix is optional.
pub fn load(path: &str) -> Result<Arc<Vec<u8>>, ResourceError> {
    let key = normalize(path);
    mem_map()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .cloned()
        .ok_or(ResourceError::MemNotFound { path: key })
}
