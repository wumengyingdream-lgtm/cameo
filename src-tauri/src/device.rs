//! Anonymous install identity — `~/.cameo/device_id`.
//!
//! A random UUID v4 generated on first launch and persisted as a plain-text
//! file (one line, no JSON wrapper). The id is sent to the Cameo cloud API as
//! `X-Device-Id` to register the install and to gate per-device rate limits
//! on Gallery requests. It is **not** tied to any account, hardware id, IP, or
//! filesystem path — reinstalling Cameo or deleting the file mints a new id.
//!
//! Storage is deliberately separate from `config.json`:
//!   - Identity vs. preferences are different concerns;
//!   - Users may reset their settings without losing identity;
//!   - "Reset device id" is a single rm of one file.

use crate::paths::cameo_data_dir;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::OnceLock;
use uuid::Uuid;

fn device_id_path() -> PathBuf {
    cameo_data_dir().join("device_id")
}

/// In-process cache so we hit disk at most once per launch.
static CACHED: OnceLock<String> = OnceLock::new();

/// Read existing id from disk, or generate + atomically write a new one.
fn load_or_create() -> Result<String> {
    let path = device_id_path();

    // Existing file: trust + sanity-check it (must look like a UUID).
    if let Ok(contents) = std::fs::read_to_string(&path) {
        let trimmed = contents.trim();
        if Uuid::parse_str(trimmed).is_ok() {
            return Ok(trimmed.to_string());
        }
        tracing::warn!(
            module = "device",
            "device_id file present but unparseable; regenerating"
        );
    }

    // Generate + atomic write.
    let id = Uuid::new_v4().to_string();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create_dir_all {}", parent.display()))?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, &id).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("rename to {}", path.display()))?;
    tracing::info!(module = "device", "minted new device_id (first launch)");
    Ok(id)
}

/// Lazy accessor used by Rust callers.
pub fn ensure() -> Result<String> {
    if let Some(id) = CACHED.get() {
        return Ok(id.clone());
    }
    let id = load_or_create()?;
    let _ = CACHED.set(id.clone());
    Ok(id)
}

// ── Tauri commands ─────────────────────────────────────────────────────────

/// Returns the cached or freshly-minted device id.
#[tauri::command]
pub fn device_id_get() -> Result<String, String> {
    ensure().map_err(|e| {
        tracing::warn!(module = "device", "device_id_get failed: {e}");
        e.to_string()
    })
}

/// Wipe the on-disk id (next call to `ensure` mints a new one). Also clears
/// the in-process cache so the change is visible immediately in this run.
///
/// Caveat: `OnceLock` can't be re-set, so callers should treat the new id as
/// effective on next app launch. We still wipe disk now so a fresh launch
/// starts clean.
#[tauri::command]
pub fn device_id_reset() -> Result<(), String> {
    let path = device_id_path();
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| {
            tracing::warn!(module = "device", "device_id_reset rm failed: {e}");
            e.to_string()
        })?;
    }
    tracing::info!(module = "device", "device_id reset (file removed)");
    Ok(())
}
