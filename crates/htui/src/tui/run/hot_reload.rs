//! Debug-only source hot reload watcher.
//!
//! Each watched source gets a small polling thread. Polling avoids a platform
//! file-event dependency; a source is emitted only after two identical reads,
//! which filters the partial writes editors commonly produce.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::source::SourceKind;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A stable update from one watched source.
#[derive(Debug)]
pub(super) enum HotReloadEvent {
    Html(String),
    Css(String),
    Js(String),
}

/// Debug-only watchers owned by the input loop.
pub(super) struct HotReloadWatcher {
    rx: Receiver<HotReloadEvent>,
    stop_flags: Vec<Arc<AtomicBool>>,
    handles: Vec<JoinHandle<()>>,
}

impl HotReloadWatcher {
    /// Stable updates destined for the input loop.
    pub(super) fn receiver(&self) -> &Receiver<HotReloadEvent> {
        &self.rx
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
    let mut stop_flags = Vec::new();
    let mut handles = Vec::new();

    for (kind, path) in watches {
        let path = PathBuf::from(path);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let tx = tx.clone();
        let result = thread::Builder::new()
            .name(format!("htui-hot-reload-{:?}", kind))
            .spawn(move || watch(kind, path, tx, thread_stop));
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
        stop_flags,
        handles,
    }
}

fn watch(kind: SourceKind, path: PathBuf, tx: Sender<HotReloadEvent>, stop: Arc<AtomicBool>) {
    let mut last: Option<String> = read(&path).ok();
    let mut pending: Option<String> = None;
    let mut warned_read_error = false;

    while !stop.load(Ordering::Relaxed) {
        thread::sleep(POLL_INTERVAL);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let current = match read(&path) {
            Ok(source) => {
                warned_read_error = false;
                Some(source)
            }
            Err(e) => {
                if !warned_read_error {
                    log::warn!("Cannot read hot-reload source {}: {e}", path.display());
                    warned_read_error = true;
                }
                None
            }
        };

        if current == last {
            pending = None;
            continue;
        }

        match pending {
            Some(previous) if Some(&previous) == current.as_ref() => {
                let Some(current) = current else {
                    unreachable!("a successful read cannot equal `None`");
                };
                last = Some(current.clone());
                pending = None;
                let event = match kind {
                    SourceKind::Html => HotReloadEvent::Html(current),
                    SourceKind::Css => HotReloadEvent::Css(current),
                    SourceKind::Js => HotReloadEvent::Js(current),
                };
                if tx.send(event).is_err() {
                    break;
                }
            }
            _ => pending = current,
        }
    }

    log::debug!("Hot-reload watcher exiting");
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
        // Let the watcher establish the initial contents as its baseline.
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
}
