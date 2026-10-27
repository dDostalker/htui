//! Debug-only source hot reload watcher.
//!
//! Each watched source gets a small polling thread. The poll first reads file
//! metadata; only when its size / modified-time signature changes does it read
//! the full file. A new payload is emitted after one more poll sees the same
//! signature, which filters transient states produced by editors.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

use crate::source::SourceKind;

const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// A stable update from one watched source.
#[derive(Debug)]
pub(super) enum HotReloadEvent {
    Html(String),
    Css(String),
    Js(String),
}

/// Cheap file fingerprint used to avoid full reads on every poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileSignature {
    modified: Option<SystemTime>,
    len: u64,
}

/// Debug-only watchers owned by the input loop.
pub(super) struct HotReloadWatcher {
    rx: Receiver<HotReloadEvent>,
    #[cfg_attr(not(test), allow(dead_code))]
    ready_rx: Receiver<()>,
    stop_flags: Vec<Arc<AtomicBool>>,
    handles: Vec<JoinHandle<()>>,
}

impl HotReloadWatcher {
    /// Stable updates destined for the input loop.
    pub(super) fn receiver(&self) -> &Receiver<HotReloadEvent> {
        &self.rx
    }

    /// Wait until the initial metadata/content baseline has been established.
    #[cfg(test)]
    pub(super) fn ready_receiver(&self) -> &Receiver<()> {
        &self.ready_rx
    }
}

impl Drop for HotReloadWatcher {
    fn drop(&mut self) {
        for stop in &self.stop_flags {
            stop.store(true, Ordering::Relaxed);
        }
        for handle in self.handles.drain(..) {
            if let Err(payload) = handle.join() {
                log::error!("Hot-reload watcher panicked: {payload:?}");
            }
        }
    }
}

/// Watch every `(kind, path)` pair and send stable updates on one channel.
pub(super) fn spawn(watches: Vec<(SourceKind, &str)>) -> HotReloadWatcher {
    let (tx, rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let mut stop_flags = Vec::new();
    let mut handles = Vec::new();

    for (kind, path) in watches {
        let path = PathBuf::from(path);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let tx = tx.clone();
        let ready_tx = ready_tx.clone();
        let result = thread::Builder::new()
            .name(format!("htui-hot-reload-{:?}", kind))
            .spawn(move || watch(kind, path, tx, ready_tx, thread_stop));
        match result {
            Ok(handle) => {
                stop_flags.push(stop);
                handles.push(handle);
            }
            Err(e) => log::error!("Failed to start hot-reload watcher: {e}"),
        }
    }

    HotReloadWatcher {
        rx,
        ready_rx,
        stop_flags,
        handles,
    }
}

fn watch(
    kind: SourceKind,
    path: PathBuf,
    tx: Sender<HotReloadEvent>,
    ready_tx: Sender<()>,
    stop: Arc<AtomicBool>,
) {
    let mut last_signature = signature(&path).ok();
    let mut last_content: Option<String> = read(&path).ok();
    // The initial baseline must exist before a caller starts a negative-test
    // window; otherwise a slow CI scheduler could make a later write look like
    // the watcher's first observation.
    let _ = ready_tx.send(());
    let mut pending: Option<String> = None;

    while !stop.load(Ordering::Relaxed) {
        thread::sleep(POLL_INTERVAL);
        if stop.load(Ordering::Relaxed) {
            break;
        }

        // Cheap pre-check: unchanged metadata does not read the payload. The
        // content comparison is still authoritative because mtime precision
        // and editor save semantics differ across platforms/filesystems.
        let current_signature = signature(&path).ok();
        if current_signature == last_signature {
            // The new signature stayed stable for one poll, so emit the full
            // payload that was read when the metadata first changed.
            if let Some(current) = pending.take() {
                let event = match kind {
                    SourceKind::Html => HotReloadEvent::Html(current),
                    SourceKind::Css => HotReloadEvent::Css(current),
                    SourceKind::Js => HotReloadEvent::Js(current),
                };
                if tx.send(event).is_err() {
                    break;
                }
            }
            continue;
        }

        let current_content = match read(&path) {
            Ok(source) => Some(source),
            Err(e) => {
                log::warn!("Cannot read hot-reload source {}: {e}", path.display());
                None
            }
        };

        match current_content {
            Some(current) => {
                let changed = last_content.as_ref() != Some(&current);
                last_content = Some(current);
                pending = if changed {
                    last_content.clone()
                } else {
                    // Metadata changed (for example a touch), but the payload
                    // is identical. Do not reset JS/document state.
                    None
                };
            }
            None => {
                last_content = None;
                pending = None;
            }
        }
        last_signature = current_signature;
    }

    log::debug!("Hot-reload watcher exiting");
}

fn signature(path: &PathBuf) -> std::io::Result<FileSignature> {
    let metadata = fs::metadata(path)?;
    Ok(FileSignature {
        modified: metadata.modified().ok(),
        len: metadata.len(),
    })
}

fn read(path: &PathBuf) -> std::io::Result<String> {
    fs::read_to_string(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn watcher_sends_only_stable_changed_contents() {
        let unique = format!(
            "htui-hot-reload-{}-{:?}.css",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("system time is after the Unix epoch")
        );
        let path = std::env::temp_dir().join(unique);
        fs::write(&path, "p { color: red; }").expect("write initial fixture");

        let watcher = spawn(vec![(
            SourceKind::Css,
            path.to_str().expect("temp path is UTF-8"),
        )]);
        let rx = watcher.receiver();
        watcher
            .ready_receiver()
            .recv_timeout(Duration::from_secs(2))
            .expect("watcher establishes its initial baseline");
        assert!(
            rx.recv_timeout(POLL_INTERVAL * 2).is_err(),
            "startup must not emit an immediate reload"
        );

        fs::write(&path, "p { color: blue; }").expect("write changed fixture");
        let changed = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("stable change reaches the watcher channel");
        assert!(matches!(changed, HotReloadEvent::Css(css) if css.contains("blue")));

        let _ = fs::remove_file(path);
        drop(watcher);
    }

    #[test]
    fn metadata_change_with_identical_content_does_not_emit() -> std::io::Result<()> {
        let unique = format!(
            "htui-hot-reload-metadata-{}-{:?}.css",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("system time is after the Unix epoch")
        );
        let path = std::env::temp_dir().join(unique);
        let css = "p { color: teal; }";
        fs::write(&path, css).expect("write initial fixture");

        let watcher = spawn(vec![(
            SourceKind::Css,
            path.to_str().expect("temp path is UTF-8"),
        )]);
        let rx = watcher.receiver();
        watcher
            .ready_receiver()
            .recv_timeout(Duration::from_secs(2))
            .expect("watcher establishes its initial baseline");
        assert!(
            rx.recv_timeout(POLL_INTERVAL * 2).is_err(),
            "startup must not emit"
        );

        // Rewrite the identical payload and force a metadata change so the
        // test does not depend on filesystem mtime precision.
        fs::write(&path, css).expect("rewrite identical fixture");
        let file = fs::OpenOptions::new().write(true).open(&path)?;
        file.set_modified(
            SystemTime::now()
                .checked_add(Duration::from_secs(1))
                .expect("valid future timestamp"),
        )?;
        assert!(
            rx.recv_timeout(POLL_INTERVAL * 3).is_err(),
            "metadata-only changes must not reset the page"
        );

        let _ = fs::remove_file(path);
        drop(watcher);
        Ok(())
    }
}
