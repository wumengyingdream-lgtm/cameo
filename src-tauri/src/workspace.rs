//! Workspaces = recently opened/added Board folders. A small global index plus
//! an app-owned default area where "+ New" creates fresh boards.
//!
//! - Index: `~/.cameo/workspaces.json` (array, atomic rewrite).
//! - Default area: `~/.cameo/workspace/<name>/` for app-created boards.
//! - Identity = the Board's stable `boardId` (meta.json). The index is keyed by
//!   it, so renames/moves never duplicate an entry.

use crate::paths::cameo_data_dir;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    pub id: String,
    pub path: String,
    pub name: String,
    /// "app" (under the default area) | "external" (a user folder).
    pub kind: String,
    pub last_opened: i64,
}

fn index_path() -> PathBuf {
    cameo_data_dir().join("workspaces.json")
}

pub fn workspace_area() -> PathBuf {
    cameo_data_dir().join("workspace")
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn read_index() -> Vec<WorkspaceEntry> {
    match std::fs::read(index_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn write_index(entries: &[WorkspaceEntry]) {
    let p = index_path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_vec_pretty(entries) {
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, &json).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

fn kind_for(path: &Path) -> String {
    if path.starts_with(workspace_area()) {
        "app".into()
    } else {
        "external".into()
    }
}

/// Recent workspaces (most-recent first), dropping any whose folder vanished.
pub fn list() -> Vec<WorkspaceEntry> {
    let mut entries: Vec<WorkspaceEntry> =
        read_index().into_iter().filter(|e| Path::new(&e.path).is_dir()).collect();
    entries.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));
    entries
}

/// Upsert (called on every open): refresh lastOpened + name/path, keyed by id.
pub fn touch(id: &str, folder: &Path, name: &str) {
    let mut entries = read_index();
    let path = folder.to_string_lossy().to_string();
    let kind = kind_for(folder);
    if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
        e.path = path;
        e.name = name.to_string();
        e.kind = kind;
        e.last_opened = now_ms();
    } else {
        entries.push(WorkspaceEntry { id: id.into(), path, name: name.into(), kind, last_opened: now_ms() });
    }
    write_index(&entries);
}

pub fn rename(id: &str, name: &str) {
    let mut entries = read_index();
    if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
        e.name = name.to_string();
        write_index(&entries);
    }
}

pub fn remove(id: &str) {
    let mut entries = read_index();
    entries.retain(|e| e.id != id);
    write_index(&entries);
}

/// Create a fresh board folder in the app area with a date+counter name.
/// Returns its absolute path (caller opens it, which records it in the index).
pub fn create() -> std::io::Result<PathBuf> {
    let area = workspace_area();
    std::fs::create_dir_all(&area)?;
    let base = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut name = base.clone();
    let mut n = 2;
    while area.join(&name).exists() {
        name = format!("{base} ({n})");
        n += 1;
    }
    let dir = area.join(&name);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
