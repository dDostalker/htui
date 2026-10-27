//! Debug-only file watcher for compile-time HTML / CSS / JS sources.
//!
//! Each watched source gets one metadata-first poll tick. The poll first
//! reads file metadata; only when its size / modified-time signature changes
//! does it read the full file. A new payload is emitted after one more poll
//! sees the same signature, which filters transient states produced by editors.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime};

const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// The source role carried by a watched path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceKind {
    Html,
    Css,
    Js,
}

/// Watch paths keyed by source role, ordered HTML, CSS, then JS.
pub(super) type SourceWatches<'a> = Vec<(SourceKind, &'a str)>;

/// A stable update from one watched source.
#[derive(Debug)]
pub(super) enum HotReloadEvent {
    Html(String),
    Css(String),
    Js(String),
}

impl HotReloadEvent {
    fn from_source(kind: SourceKind, content: String) -> Self {
        match kind {
            SourceKind::Html => Self::Html(content),
            SourceKind::Css => Self::Css(content),
            SourceKind::Js => Self::Js(content),
        }
    }
}

/// Cheap file fingerprint used to avoid full reads on every poll.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileSignature {
    modified: Option<SystemTime>,
    len: u64,
}

impl FileSignature {
    fn new(path: &Path) -> std::io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        })
    }
}

/// Mutable polling state for one watched file.
struct WatchState {
    kind: SourceKind,
    path: PathBuf,
    signature: Option<FileSignature>,
    content: Option<String>,
    pending: Option<String>,
}

impl WatchState {
    fn new(kind: SourceKind, path: &str) -> Self {
        let path = PathBuf::from(path);
        Self {
            kind,
            signature: FileSignature::new(&path).ok(),
            content: read(&path).ok(),
            path,
            pending: None,
        }
    }

    /// Poll once; return `false` when the event receiver is gone.
    fn poll(&mut self, tx: &Sender<HotReloadEvent>) -> bool {
        // Cheap pre-check: unchanged metadata does not read the payload. The
        // content comparison is still authoritative because mtime precision
        // and editor save semantics differ across platforms/filesystems.
        let signature = FileSignature::new(&self.path).ok();
        if signature == self.signature {
            // The new signature stayed stable for one poll, so emit the full
            // payload that was read when the metadata first changed.
            if let Some(content) = self.pending.take()
                && tx
                    .send(HotReloadEvent::from_source(self.kind, content))
                    .is_err()
            {
                return false;
            }
            return true;
        }

        let content = match read(&self.path) {
            Ok(content) => Some(content),
            Err(e) => {
                log::warn!("Cannot read hot-reload source {}: {e}", self.path.display());
                None
            }
        };
        // Metadata changed (for example a touch), but an identical payload
        // must not reset JS/document state.
        self.pending = if content.as_ref() == self.content.as_ref() {
            None
        } else {
            content.clone()
        };
        self.content = content;
        self.signature = signature;
        true
    }
}

/// Debug-only watcher owned by the input loop.
pub(super) struct HotReloadWatcher {
    rx: Receiver<HotReloadEvent>,
    #[cfg_attr(not(test), allow(dead_code))]
    ready_rx: Receiver<()>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
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
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take()
            && let Err(payload) = handle.join()
        {
            log::error!("Hot-reload watcher panicked: {payload:?}");
        }
    }
}

/// Watch every selected source and send stable updates on one channel.
pub(super) fn spawn(watches: SourceWatches<'_>) -> HotReloadWatcher {
    let (tx, rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let watches = watches
        .into_iter()
        .map(|(kind, path)| (kind, path.to_owned()))
        .collect::<Vec<_>>();

    let handle = thread::Builder::new()
        .name("htui-hot-reload".to_string())
        .spawn(move || {
            let mut states = watches
                .into_iter()
                .map(|(kind, path)| WatchState::new(kind, &path))
                .collect::<Vec<_>>();

            // The baseline must exist before a caller starts a negative-test
            // window; otherwise a slow scheduler could make a later write look
            // like the watcher's first observation.
            let _ = ready_tx.send(());

            while !thread_stop.load(Ordering::Relaxed) {
                thread::sleep(POLL_INTERVAL);
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                if !states.iter_mut().all(|state| state.poll(&tx)) {
                    break;
                }
            }
        });

    let handle = match handle {
        Ok(handle) => Some(handle),
        Err(e) => {
            log::error!("Failed to start hot-reload watcher: {e}");
            None
        }
    };

    HotReloadWatcher {
        rx,
        ready_rx,
        stop,
        handle,
    }
}

fn read(path: &Path) -> std::io::Result<String> {
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
