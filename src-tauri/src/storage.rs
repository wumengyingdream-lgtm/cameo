//! Atomic load/save of `<folder>/.cameo/board.json`.

use crate::model::{BoardDoc, BoardMeta};
use crate::paths::{board_doc_path, board_meta_path};
use anyhow::{Context, Result};
use std::path::Path;

pub fn load_meta(folder: &Path) -> BoardMeta {
    match std::fs::read(board_meta_path(folder)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => BoardMeta::default(),
    }
}

pub fn save_meta(folder: &Path, meta: &BoardMeta) {
    if let Ok(json) = serde_json::to_vec_pretty(meta) {
        let _ = std::fs::write(board_meta_path(folder), json);
    }
}

/// Folder basename, used as the default workspace name.
pub fn folder_name(folder: &Path) -> String {
    folder
        .file_name()
        .and_then(|n| n.to_str())
        .map(String::from)
        .unwrap_or_else(|| folder.to_string_lossy().to_string())
}

pub fn load_board_doc(folder: &Path) -> BoardDoc {
    let p = board_doc_path(folder);
    match std::fs::read(&p) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!(
                module = "storage",
                path = %p.display(),
                "board.json parse failed ({e}); starting from a fresh doc"
            );
            BoardDoc::default()
        }),
        Err(_) => BoardDoc::default(),
    }
}

/// Write atomically: temp file + rename, so a crash mid-write never corrupts
/// the doc.
pub fn save_board_doc(folder: &Path, doc: &BoardDoc) -> Result<()> {
    let p = board_doc_path(folder);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(doc)?;
    std::fs::write(&tmp, &json).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &p).with_context(|| format!("rename to {}", p.display()))?;
    Ok(())
}
