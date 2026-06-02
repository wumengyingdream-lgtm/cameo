//! Managed external tools.
//!
//! A generic resolver for native CLI tools Cameo can lean on: **detect first**
//! (the user's own install on PATH — same stance as the Codex CLI), and only if
//! missing, **silently download** a pinned build into `~/.cameo/bin/` and verify
//! it before it's ever executed. ffmpeg/ffprobe is the first (and currently
//! only) instance — see [`ffmpeg`]. New tools should grow as siblings here
//! rather than hard-coding tool names across the codebase (decision E4).
//!
//! The managed bin dir is APPENDED (last resort) to the Codex sidecar PATH (see
//! `codex::build_augmented_path`), so a tool Cameo downloaded is found by BOTH
//! Cameo's own probes and the agent's shell — but the user's own install on
//! PATH always wins (detect-first). One wiring, every consumer.

pub mod ffmpeg;

use std::path::PathBuf;

/// `~/.cameo/bin` — where managed tool binaries live.
pub fn bin_dir() -> PathBuf {
    crate::paths::cameo_bin_dir()
}
