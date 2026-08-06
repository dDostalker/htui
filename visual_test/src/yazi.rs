//! Yazi-style file manager demo: a real directory listing exposed to the
//! page through a `#[htui::command]` host function.
//!
//! The demo runs with the process CWD set to the visual-test crate root (see
//! `main.rs`), so `listDir(".")` lists the crate's own sources.

use std::fs;

use htui::events::EventBindings;
use htui::IntoJs;

/// One entry in a directory listing, serialized to a JS object.
#[derive(IntoJs)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
}

/// Everything the page calls, registered by `#[htui::commands]`.
#[htui::commands]
mod commands {
    use super::*;

    /// `listDir(path)` — list a directory as `[{ name, is_dir, size }]`,
    /// directories first, then files, both alphabetical (case-insensitive).
    /// Errors (missing / unreadable directory) surface as an empty list.
    #[htui::command(name = "listDir")]
    pub(super) fn list_dir(path: String) -> Vec<FileEntry> {
        let mut entries: Vec<FileEntry> = match fs::read_dir(&path) {
            Ok(rd) => rd
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    let size = if is_dir {
                        0
                    } else {
                        entry.metadata().map(|m| m.len()).unwrap_or(0)
                    };
                    Some(FileEntry {
                        name,
                        is_dir,
                        size,
                    })
                })
                .collect(),
            Err(e) => {
                log::warn!("listDir({path:?}) failed: {e}");
                return Vec::new();
            }
        };
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        entries
    }
}

/// Bindings for the page: the `listDir` command only.
pub fn bindings() -> EventBindings {
    let mut bindings = EventBindings::default();
    commands::register(&mut bindings);
    bindings
}
