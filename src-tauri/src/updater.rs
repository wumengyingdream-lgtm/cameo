//! Auto-updater for the Cameo desktop client.
//!
//! Background flow (silent download, on-demand install):
//!   1. App boot + 60s delay → check the platform-specific manifest at
//!      `https://r.cameo.ink/update/<target>-<arch>.json`.
//!   2. If a newer version is published, the binary is downloaded silently —
//!      the user sees no banner, no modal, nothing.
//!   3. Download completes → frontend gets `updater:ready-to-restart` event →
//!      titlebar shows the "重启更新" button.
//!   4. User clicks → graceful Codex sidecar shutdown → apply update + relaunch
//!      (macOS) or fire the NSIS installer (Windows).
//!
//! Platform split (the install path is the only real difference):
//!   • macOS: `Update::download_and_install()` swaps `/Applications/Cameo.app`
//!     atomically; relaunch() picks up the new bundle. One-phase.
//!   • Windows: `Update::download()` streams bytes to `~/.cameo/pending_update.bin`
//!     with metadata in `pending_update.json`. Install runs only when the user
//!     clicks the titlebar button — at that point we re-resolve `Update` (cached
//!     from the original check, with 3-retry fallback) and call `install(bytes)`,
//!     which spawns the NSIS installer and `exit(0)`s.
//!
//! Why cache the `Update` object? `Update::install(bytes)` requires an `Update`,
//! and the only public way to obtain one is `updater.check().await` — a network
//! hop. On Windows that hop runs at click-time, and a flaky/blocked network
//! silently fails the install (the button does nothing). Caching the object at
//! `check()` time lets the click-handler skip the network entirely. Bytes are
//! signature-verified at download, so this is strictly safer than rechecking.

use crate::codex::CodexRegistry;
use crate::config;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

#[cfg(target_os = "windows")]
use crate::paths::{cameo_data_dir, ensure_data_layout};
#[cfg(target_os = "windows")]
use serde::Deserialize;

/// Global flag — prevents two concurrent checks from racing on the download path.
static UPDATE_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// The version of the bytes that completed a silent background download.
/// Mirrors the disk-cached version on Windows; on macOS it's the version
/// that was just installed in-place and is awaiting a relaunch.
static DOWNLOADED_VERSION: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// The most recent `Update` from a successful `check()` this session.
/// Used by the Windows install path to skip the network round-trip — see the
/// module doc above.
static LATEST_UPDATE: std::sync::Mutex<Option<Update>> = std::sync::Mutex::new(None);

fn cache_update(update: Update) {
    if let Ok(mut g) = LATEST_UPDATE.lock() {
        *g = Some(update);
    }
}

#[cfg(target_os = "windows")]
fn take_cached_update(wanted_version: &str) -> Option<Update> {
    let mut g = LATEST_UPDATE.lock().ok()?;
    let same = g.as_ref().map(|u| u.version.as_str()) == Some(wanted_version);
    if !same {
        return None;
    }
    g.take()
}

#[cfg(target_os = "windows")]
fn pending_bin_path() -> std::path::PathBuf {
    cameo_data_dir().join("pending_update.bin")
}

#[cfg(target_os = "windows")]
fn pending_meta_path() -> std::path::PathBuf {
    cameo_data_dir().join("pending_update.json")
}

#[cfg(target_os = "windows")]
#[derive(Serialize, Deserialize)]
struct PendingMeta {
    version: String,
    notes: Option<String>,
}

// ── proxy injection ─────────────────────────────────────────────────────────
//
// The Codex sidecar uses the same proxy settings (proxy.rs::ProxySettings); the
// updater piggybacks on that config so a user with a proxy isn't stuck unable
// to fetch the manifest. 15s per-request timeout keeps us off the "hang on a
// blackholed connection" path.
fn build_updater_with_proxy(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let mut builder = app
        .updater_builder()
        .timeout(Duration::from_secs(15));
    let cfg = config::load();
    if cfg.proxy.enabled {
        match cfg.proxy.proxy_url() {
            Ok(url_str) => match reqwest::Url::parse(&url_str) {
                Ok(url) => {
                    tracing::info!(module = "updater", proxy = %url_str, "applying proxy to updater HTTP client");
                    builder = builder.proxy(url);
                }
                Err(e) => {
                    tracing::warn!(module = "updater", "proxy url parse failed ({e}); continuing without");
                }
            },
            Err(e) => {
                tracing::warn!(module = "updater", "proxy enabled but invalid ({e}); continuing without");
            }
        }
    }
    builder.build().map_err(|e| {
        tracing::warn!(module = "updater", "build updater failed: {e}");
        e.to_string()
    })
}

// ── startup check entry point ───────────────────────────────────────────────
//
// Called from `lib.rs` setup. Sleeps 60s so the app's first launch isn't burdened
// by a network round-trip, then runs the silent download path. Errors are
// swallowed — failed update checks are not the user's problem.
pub async fn check_update_on_startup(app: AppHandle) {
    tracing::info!(module = "updater", "scheduling startup update check (60s delay)");
    tokio::time::sleep(Duration::from_secs(60)).await;
    if let Err(e) = check_and_download_silently(&app).await {
        tracing::warn!(module = "updater", "startup update check failed: {e}");
    }
}

// ── silent check + download ─────────────────────────────────────────────────
//
// One-shot worker. Returns `Some(version)` if a newer version was downloaded;
// `None` if up-to-date or already-downloaded-this-session.
async fn check_and_download_silently(app: &AppHandle) -> Result<Option<String>, String> {
    if UPDATE_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        tracing::info!(module = "updater", "update already in progress, skipping");
        return Ok(None);
    }
    let result = check_and_download_silently_inner(app).await;
    UPDATE_IN_PROGRESS.store(false, Ordering::SeqCst);
    result
}

async fn check_and_download_silently_inner(app: &AppHandle) -> Result<Option<String>, String> {
    let updater = build_updater_with_proxy(app)?;
    let update_opt = updater.check().await.map_err(|e| e.to_string())?;
    let Some(update) = update_opt else {
        tracing::info!(module = "updater", "no update available");
        return Ok(None);
    };

    let new_version = update.version.clone();
    tracing::info!(module = "updater", version = %new_version, "update available — starting silent download");

    // Skip re-download if we already have these bytes this session.
    if let Ok(g) = DOWNLOADED_VERSION.lock() {
        if g.as_deref() == Some(new_version.as_str()) {
            tracing::info!(module = "updater", version = %new_version, "already downloaded this session — skipping");
            return Ok(None);
        }
    }

    // Cache the Update object NOW so the click-handler doesn't need the network.
    cache_update(update.clone());

    let _ = app.emit("updater:download-started", &new_version);

    // Progress emitter shared by both platforms. Captured by closure into the
    // updater's on_chunk callback. Throttled to every ~2% to avoid IPC spam.
    let progress_app = app.clone();
    let last_pct = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let last_pct_cb = last_pct.clone();
    let on_chunk = move |downloaded: usize, content_length: Option<u64>| {
        if let Some(total) = content_length {
            let pct = (downloaded as u64 * 100) / total.max(1);
            let prev = last_pct_cb.load(Ordering::Relaxed);
            if pct >= prev + 2 || pct == 100 {
                last_pct_cb.store(pct, Ordering::Relaxed);
                let _ = progress_app.emit(
                    "updater:download-progress",
                    DownloadProgress { downloaded: downloaded as u64, total },
                );
            }
        }
    };

    #[cfg(target_os = "macos")]
    {
        // mac: one-phase — bytes stream straight into /Applications/Cameo.app
        // (atomic replacement). The running process keeps running on the old
        // bundle; relaunch picks up the new one.
        match update.download_and_install(on_chunk, || {}).await {
            Ok(_) => {
                tracing::info!(module = "updater", version = %new_version, "mac install complete; awaiting user relaunch");
                if let Ok(mut g) = DOWNLOADED_VERSION.lock() {
                    *g = Some(new_version.clone());
                }
                let _ = app.emit("updater:ready-to-restart", &new_version);
                Ok(Some(new_version))
            }
            Err(e) => {
                tracing::warn!(module = "updater", "mac download_and_install failed: {e}");
                let _ = app.emit("updater:download-failed", &e.to_string());
                Err(e.to_string())
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        // win: two-phase — accumulate bytes, write atomically to disk, defer
        // the NSIS install until the user clicks "重启更新". This protects the
        // running app: NSIS does `exit(0)` to swap files, so kicking it off
        // mid-session would kill an unsaved chat.
        let bytes = update
            .download(on_chunk, || {})
            .await
            .map_err(|e| {
                tracing::warn!(module = "updater", "windows download failed: {e}");
                let _ = app.emit("updater:download-failed", &e.to_string());
                e.to_string()
            })?;

        // Persist bytes via tmp + rename so a crash mid-write doesn't leave a
        // half-baked installer. Meta file written second; readers tolerate it
        // being absent (treated as no pending update).
        ensure_data_layout().map_err(|e| e.to_string())?;
        let dst = pending_bin_path();
        let tmp = dst.with_extension("bin.tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &dst).map_err(|e| e.to_string())?;
        let meta = PendingMeta {
            version: new_version.clone(),
            notes: Some(update.body.clone().unwrap_or_default()),
        };
        std::fs::write(
            pending_meta_path(),
            serde_json::to_vec(&meta).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;

        if let Ok(mut g) = DOWNLOADED_VERSION.lock() {
            *g = Some(new_version.clone());
        }

        tracing::info!(
            module = "updater",
            version = %new_version,
            bytes = bytes.len(),
            "windows update bytes persisted; awaiting user click"
        );
        let _ = app.emit("updater:ready-to-restart", &new_version);
        Ok(Some(new_version))
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        // Linux / others: tauri-plugin-updater supports AppImage on linux but
        // we don't ship that today. No-op rather than failing.
        let _ = app.emit("updater:download-failed", "unsupported platform");
        Ok(None)
    }
}

#[derive(Serialize, Clone)]
struct DownloadProgress {
    downloaded: u64,
    total: u64,
}

// ── manual check (settings panel) ──────────────────────────────────────────
//
// Same as the startup path but without the 60s delay. Returns whether an update
// was found and download was attempted (for inline feedback in Settings).
#[tauri::command]
pub async fn check_and_download_update(app: AppHandle) -> Result<bool, String> {
    match check_and_download_silently(&app).await? {
        Some(_) => Ok(true),
        None => Ok(false),
    }
}

// ── windows: pending bytes on startup ───────────────────────────────────────
//
// Read on app boot (from lib.rs setup) to detect a previously-downloaded update
// the user hasn't installed yet. Emits the version up to the frontend so the
// "重启更新" button reappears without re-checking the server.
#[tauri::command]
pub fn check_pending_update() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        let meta_path = pending_meta_path();
        let bin_path = pending_bin_path();
        if !meta_path.exists() || !bin_path.exists() {
            return None;
        }
        let bytes = std::fs::read(&meta_path).ok()?;
        let meta: PendingMeta = serde_json::from_slice(&bytes).ok()?;
        let current = env!("CARGO_PKG_VERSION");
        if version_gt(&meta.version, current) {
            tracing::info!(module = "updater", version = %meta.version, "found pending update on disk");
            Some(meta.version)
        } else {
            // Pending update is older than what's installed (manual upgrade?) —
            // clear it so a stale install doesn't downgrade the user.
            let _ = std::fs::remove_file(&bin_path);
            let _ = std::fs::remove_file(&meta_path);
            None
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Naive semver comparison — sufficient for `0.0.2 > 0.0.1` style versions.
/// Doesn't understand pre-release suffixes; we don't ship channels.
#[cfg(target_os = "windows")]
fn version_gt(a: &str, b: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> { s.split('.').filter_map(|p| p.parse().ok()).collect() };
    let av = parse(a);
    let bv = parse(b);
    for i in 0..av.len().max(bv.len()) {
        let ax = av.get(i).copied().unwrap_or(0);
        let bx = bv.get(i).copied().unwrap_or(0);
        if ax != bx {
            return ax > bx;
        }
    }
    false
}

// ── windows: actually install ──────────────────────────────────────────────
//
// Triggered by the frontend "重启更新" click. Resolves the `Update` (cached if
// possible, else re-check with 3-attempt fallback), calls `install(bytes)`.
// NSIS takes over from there.
#[cfg(target_os = "windows")]
#[tauri::command]
pub async fn install_pending_update(
    app: AppHandle,
    codex: State<'_, Arc<CodexRegistry>>,
) -> Result<(), String> {
    let bin = pending_bin_path();
    let meta = pending_meta_path();
    if !bin.exists() || !meta.exists() {
        return Err("no pending update on disk".into());
    }
    let bytes = std::fs::read(&bin).map_err(|e| e.to_string())?;
    let meta_bytes = std::fs::read(&meta).map_err(|e| e.to_string())?;
    let meta: PendingMeta = serde_json::from_slice(&meta_bytes).map_err(|e| e.to_string())?;

    // Resolve an Update object — cache hit avoids the network entirely.
    let update = match take_cached_update(&meta.version) {
        Some(u) => {
            tracing::info!(module = "updater", version = %meta.version, "using cached Update for install");
            u
        }
        None => {
            tracing::info!(module = "updater", version = %meta.version, "cache miss; re-checking server (with retry)");
            let mut last_err = String::from("unknown");
            let mut got = None;
            for attempt in 1..=3 {
                let updater = build_updater_with_proxy(&app)?;
                match updater.check().await {
                    Ok(Some(u)) if u.version == meta.version => {
                        got = Some(u);
                        break;
                    }
                    Ok(Some(u)) => {
                        last_err = format!("server returned different version {} (expected {})", u.version, meta.version);
                    }
                    Ok(None) => {
                        last_err = "server reports no update available".into();
                    }
                    Err(e) => {
                        last_err = e.to_string();
                    }
                }
                if attempt < 3 {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
            got.ok_or_else(|| format!("update resolve failed after 3 attempts: {last_err}"))?
        }
    };

    // Shut down the Codex sidecar BEFORE NSIS takes the process down with exit(0),
    // so we leave a clean process tree (no orphaned codex children).
    tracing::info!(module = "updater", "shutting down codex sidecar before install");
    codex.kill_all_sync();

    // Hand off to NSIS. install() spawns the installer and calls exit(0).
    update
        .install(bytes)
        .map_err(|e| {
            tracing::warn!(module = "updater", "windows install failed: {e}");
            e.to_string()
        })?;

    Ok(())
}

/// macOS install + relaunch (single command — bytes are already on disk in
/// `/Applications/Cameo.app` from the silent download). Cleanly stops the
/// codex sidecar before `relaunch()` so the new process can spawn its own.
#[cfg(target_os = "macos")]
#[tauri::command]
pub async fn install_pending_update(
    app: AppHandle,
    codex: State<'_, Arc<CodexRegistry>>,
) -> Result<(), String> {
    tracing::info!(module = "updater", "shutting down codex sidecar before relaunch");
    codex.kill_all_sync();
    app.restart();
}
