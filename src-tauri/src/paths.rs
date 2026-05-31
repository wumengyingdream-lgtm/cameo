//! Filesystem layout.
//!
//! Two distinct `.cameo` locations — don't confuse them:
//!
//! 1. **Global app dir** `~/.cameo/` — app-level state that is NOT tied to any
//!    one Board: logs, the recent-Boards index, app settings. Overridable with
//!    `CAMEO_HOME` (tests / portable installs).
//!
//! 2. **Per-Board sidecar** `<board-folder>/.cameo/` — everything that belongs
//!    to one Board: `board.json` (Placements / Annotations / layout),
//!    `meta.json` (threadId / runtime / settings), `session.jsonl` (timeline),
//!    `thumbs/`, and dispatch temp images. Lives INSIDE the user's folder so the
//!    Board is self-contained and portable (like `.git`). The Codex agent's cwd
//!    is the folder itself; it is told not to touch `.cameo/` (dot-prefix +
//!    system-prompt constraint) but CAN read under it (sandbox = workspace-write
//!    rooted at the folder) — which is why dispatch temp images live here
//!    (decision D5).

use std::path::{Path, PathBuf};

// ── Global app dir ─────────────────────────────────────────────────────────

pub fn cameo_data_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("CAMEO_HOME") {
        return PathBuf::from(custom);
    }
    dirs::home_dir().expect("home dir").join(".cameo")
}

pub fn cameo_logs_dir() -> PathBuf {
    cameo_data_dir().join("logs")
}

/// Newline-delimited JSON index of recently opened Boards (path + last-opened).
pub fn boards_index_path() -> PathBuf {
    cameo_data_dir().join("boards.jsonl")
}

/// Global app config (network proxy etc.) — `~/.cameo/config.json`.
pub fn app_config_path() -> PathBuf {
    cameo_data_dir().join("config.json")
}

pub fn ensure_data_layout() -> std::io::Result<()> {
    std::fs::create_dir_all(cameo_data_dir())?;
    std::fs::create_dir_all(cameo_logs_dir())?;
    Ok(())
}

// ── Per-Board sidecar (inside the user's folder) ─────────────────────────────

/// `<folder>/.cameo`
pub fn board_sidecar_dir(folder: &Path) -> PathBuf {
    folder.join(".cameo")
}

/// `<folder>/.cameo/board.json` — Placements / Annotations / layout.
pub fn board_doc_path(folder: &Path) -> PathBuf {
    board_sidecar_dir(folder).join("board.json")
}

/// `<folder>/.cameo/meta.json` — threadId / runtime / settings.
pub fn board_meta_path(folder: &Path) -> PathBuf {
    board_sidecar_dir(folder).join("meta.json")
}

/// `<folder>/.cameo/sessions.json` — the session index (v0.0.2 multi-session).
pub fn board_sessions_doc(folder: &Path) -> PathBuf {
    board_sidecar_dir(folder).join("sessions.json")
}

/// `<folder>/.cameo/sessions/` — per-session message timelines.
pub fn board_sessions_dir(folder: &Path) -> PathBuf {
    board_sidecar_dir(folder).join("sessions")
}

/// `<folder>/.cameo/sessions/<id>.jsonl` — one session's append-only timeline.
pub fn board_session_timeline(folder: &Path, session_id: &str) -> PathBuf {
    board_sessions_dir(folder).join(format!("{session_id}.jsonl"))
}

/// `<folder>/.cameo/thumbs` — thumbnail cache.
pub fn board_thumbs_dir(folder: &Path) -> PathBuf {
    board_sidecar_dir(folder).join("thumbs")
}

/// `<folder>/.cameo/posters` — extracted video poster frames (blake3-named JPEG),
/// served via the Cameo image protocol as the canvas still for a video Asset.
pub fn board_posters_dir(folder: &Path) -> PathBuf {
    board_sidecar_dir(folder).join("posters")
}

/// `~/.cameo/bin` — managed external tools (ffmpeg/ffprobe). Prepended to the
/// Codex sidecar PATH so the agent finds tools Cameo downloaded (see tools::ffmpeg).
pub fn cameo_bin_dir() -> PathBuf {
    cameo_data_dir().join("bin")
}

/// `<folder>/.cameo/tmp` — dispatch temp images (clean + overlay). Inside the
/// workspace so the Codex sandbox can read them (decision D5).
pub fn board_tmp_dir(folder: &Path) -> PathBuf {
    board_sidecar_dir(folder).join("tmp")
}

/// Create the per-Board sidecar layout. Idempotent.
pub fn ensure_board_sidecar(folder: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(board_sidecar_dir(folder))?;
    std::fs::create_dir_all(board_thumbs_dir(folder))?;
    std::fs::create_dir_all(board_posters_dir(folder))?;
    std::fs::create_dir_all(board_tmp_dir(folder))?;
    std::fs::create_dir_all(board_sessions_dir(folder))?;
    Ok(())
}
