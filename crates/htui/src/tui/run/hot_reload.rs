//! Debug-only whole-document hot reload watcher.
//!
//! The watcher deliberately uses short polling instead of a platform file-event
//! dependency. A 100 ms cadence is fast enough for an HTML edit loop, works
//! identically on Windows / Unix / network filesystems, and keeps the public
//! dependency surface unchanged. A change is emitted only after two identical
//! reads, which filters the partial writes editors commonly produce.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A debug-only watcher owned by the input loop.
pub(super) struct HotReloadWatcher {
    rx: Receiver<String>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl HotReloadWatcher {
    /// The stable file updates destined for the input loop.
    pub(super) fn receiver(&self) -> &Receiver<String> {
        &self.rx
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

/// Watch `path` and send each stable new file contents on the returned channel.
pub(super) fn spawn(path: &str) -> HotReloadWatcher {
    let (tx, rx) = mpsc::channel();
    let path = PathBuf::from(path);
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let result = thread::Builder::new()
        .name("htui-hot-reload".to_owned())
        .spawn(move || watch(path, tx, thread_stop));
    let handle = match result {
        Ok(handle) => Some(handle),
        Err(e) => {
            log::error!("Failed to start hot-reload watcher: {e}");
            None
        }
    };
    HotReloadWatcher { rx, stop, handle }
}

fn watch(path: PathBuf, tx: Sender<String>, stop: Arc<AtomicBool>) {
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
                if tx.send(current).is_err() {
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
            "htui-hot-reload-{}-{:?}.html",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("system time is after the Unix epoch")
        );
        let path = std::env::temp_dir().join(unique);
        fs::write(&path, "<p>initial</p>").expect("write initial fixture");

        let watcher = spawn(path.to_str().expect("temp path is UTF-8"));
        let rx = watcher.receiver();
        // Let the watcher establish the initial contents as its baseline.
        assert!(
            rx.recv_timeout(POLL_INTERVAL * 2).is_err(),
            "startup must not emit an immediate reload"
        );

        fs::write(&path, "<p id=\"fresh\">changed</p>").expect("write changed fixture");
        let changed = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("stable change reaches the watcher channel");
        assert!(changed.contains("fresh"));

        let _ = fs::remove_file(path);
        drop(watcher);
    }
}
