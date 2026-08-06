//! File-based logging initialization for `htui`.
//!
//! `init_file_logger` is the only public entry point: it appends log lines
//! to a caller-supplied file and installs an `env_logger` as the global
//! `log` recorder. The global recorder can only be set once per process, so
//! calling `init_file_logger` more than once returns an error on the second
//! and subsequent calls.
//!
//! `default_level()` returns a build-profile-dependent [`LevelFilter`] that
//! pairs with the `log` crate's `release_max_level_warn` Cargo feature
//! (enabled in `Cargo.toml`): `debug!` / `info!` calls are compiled out of
//! release binaries entirely, and the runtime filter is set to `Warn`.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use log::LevelFilter;

/// Returns the default [`LevelFilter`] for the current build profile.
///
/// In debug builds this is [`LevelFilter::Debug`]; in release builds it is
/// [`LevelFilter::Warn`].
pub fn default_level() -> LevelFilter {
    if cfg!(debug_assertions) {
        LevelFilter::Debug
    } else {
        LevelFilter::Warn
    }
}

/// `Write` adapter that serializes access to an inner writer through a
/// `Mutex`. Needed because `env_logger::Target::Pipe` requires
/// `Box<dyn Write + Send + Sync>`, and `File` is `Send` but not `Sync`.
struct MutexWriter<W: Write + Send> {
    inner: Mutex<W>,
}

impl<W: Write + Send> MutexWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner: Mutex::new(inner),
        }
    }

    fn lock(&self) -> MutexGuard<'_, W> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl<W: Write + Send> Write for MutexWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.lock().write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.lock().flush()
    }
}

fn build_builder<W: Write + Send + 'static>(writer: W, level: LevelFilter) -> env_logger::Builder {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(level.as_str()));
    builder.target(env_logger::Target::Pipe(Box::new(MutexWriter::new(writer))));
    builder
}

/// Errors returned by [`init_file_logger`].
#[derive(Debug, thiserror::Error)]
pub enum LogInitError {
    /// The parent directory of the log file could not be created.
    #[error("failed to create log directory: {path}")]
    CreateDir {
        /// The directory that could not be created.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The log file could not be opened for appending.
    #[error("failed to open log file: {path}")]
    OpenFile {
        /// The file that could not be opened.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A global logger is already installed; only one can exist per process.
    #[error("global logger is already initialized")]
    AlreadyInitialized,
}

/// Initialize a file-based logger that appends log lines to `path`.
///
/// The file is opened in append mode; missing parent directories are created
/// automatically. If `level` is `None`, [`default_level`] is used.
///
/// Returns an error if the file cannot be opened, or if the global logger
/// has already been initialized.
pub fn init_file_logger(
    path: impl AsRef<Path>,
    level: Option<LevelFilter>,
) -> Result<(), LogInitError> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|source| LogInitError::CreateDir {
                path: parent.display().to_string(),
                source,
            })?;
        }
    }
    let file = File::options()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|source| LogInitError::OpenFile {
            path: path.display().to_string(),
            source,
        })?;

    build_builder(file, level.unwrap_or_else(default_level))
        .try_init()
        .map_err(|_| LogInitError::AlreadyInitialized)?;
    Ok(())
}
