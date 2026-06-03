//! Managed ffmpeg/ffprobe: detect → (silent download) → verify → probe/poster.
//!
//! Resolution order (decision E1 / review §4 — detect-first, the user's own
//! install wins):
//!   1. system PATH + the same broad augmentation Codex uses (brew, scoop, …)
//!   2. `~/.cameo/bin/` (a build Cameo downloaded earlier) — appended LAST to
//!      that augmented PATH, so it's only the fallback
//!   3. neither → `Missing`; the UI offers a one-click install
//!
//! Install (decision D1 — R2-mirrored pinned build): fetch the signed-origin
//! manifest over the proxied client (`net::client`), download each binary while
//! hashing it, verify the blake3 pin BEFORE the bytes are ever marked
//! executable, install atomically into `~/.cameo/bin/`, and on macOS ad-hoc
//! re-sign so an unsigned arm64 binary isn't `killed:9`.
//!
//! Everything funnels through [`resolved`] so callers (asset minting, the status
//! command) never re-implement detection.

use crate::tools::bin_dir;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

const RUN_TIMEOUT: Duration = Duration::from_secs(8);
/// ffprobe/extract_poster get a longer ceiling than the `-version` liveness
/// probe — real work, but still bounded so a corrupt/hung file or a stalled
/// network-mounted path can never block import / backfill / open_board forever.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const MANIFEST_URL: &str = "https://r.cameo.ink/tools/ffmpeg/manifest.json";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// One install runs at a time (the UI button + an auto-trigger could both fire).
static INSTALLING: AtomicBool = AtomicBool::new(false);
/// Cache of a SUCCESSFUL resolution only (`Some(Tools)`); `None` = not resolved
/// yet OR known-missing, both of which re-probe. Invalidated after install so
/// the freshly-downloaded pair is re-resolved.
static RESOLVED: Mutex<Option<Tools>> = Mutex::new(None);

/// Resolved tool paths (both must be present to be usable).
#[derive(Debug, Clone)]
pub struct Tools {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

/// Status reported to the UI (Agent panel / Settings).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegStatus {
    /// "ready" | "missing" | "installing" | "failed"
    pub state: String,
    pub ffmpeg_path: Option<String>,
    pub ffprobe_path: Option<String>,
    /// `ffmpeg -version` first line, when ready.
    pub version: Option<String>,
    pub error: Option<String>,
}

/// Probed video metadata (ffprobe). Missing fields stay `None`.
#[derive(Debug, Clone, Default)]
pub struct VideoMeta {
    pub width: u32,
    pub height: u32,
    pub duration_ms: Option<f64>,
    pub fps: Option<f64>,
    pub has_audio: bool,
}

#[cfg(windows)]
fn exe(name: &str) -> String {
    format!("{name}.exe")
}
#[cfg(not(windows))]
fn exe(name: &str) -> String {
    name.to_string()
}

/// `<bin> -version` succeeds within the timeout (proves it's executable on this
/// arch — catches `killed:9` unsigned mac binaries and arch mismatches).
fn runs_ok(bin: &Path) -> bool {
    let mut cmd = Command::new(bin);
    cmd.arg("-version");
    crate::process::hide_console_window(&mut cmd);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    match cmd.spawn() {
        Ok(mut child) => {
            let deadline = Instant::now() + RUN_TIMEOUT;
            loop {
                match child.try_wait() {
                    Ok(Some(status)) => return status.success(),
                    Ok(None) if Instant::now() < deadline => {
                        std::thread::sleep(Duration::from_millis(30))
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        return false;
                    }
                }
            }
        }
        Err(_) => false,
    }
}

/// Run a command to completion with a deadline, killing it (and reaping) on
/// timeout so a hung ffmpeg/ffprobe never blocks the calling worker thread.
/// Returns `(exit status, captured stdout)`, or `None` on spawn failure/timeout.
/// The caller must have configured `stdout(piped())` + `stderr(null())`.
///
/// stdout is drained on a SEPARATE thread (not after exit): ffprobe's JSON can
/// exceed the OS pipe buffer, and a child that blocks writing stdout would never
/// reach exit — `try_wait` would never see completion and the deadline would
/// kill a perfectly healthy process. Concurrent draining avoids that deadlock.
fn output_with_deadline(
    mut cmd: Command,
    timeout: Duration,
) -> Option<(std::process::ExitStatus, Vec<u8>)> {
    cmd.stdin(std::process::Stdio::null());
    let mut child = cmd.spawn().ok()?;
    let stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut s) = stdout {
            use std::io::Read;
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(30))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join(); // child killed → its stdout closes → reader returns
                return None;
            }
        }
    };
    let stdout_bytes = reader.join().ok()?;
    Some((status, stdout_bytes))
}

/// Find one tool, **detect-first** (decision E1 / review §4): the user's own
/// install on PATH wins; only if none is found do we fall back to the managed
/// `~/.cameo/bin` copy Cameo downloaded. This matches the locked decision and
/// the Codex-CLI stance ("use the user's own ffmpeg"); the managed bin dir is
/// appended (not prepended) to the augmented PATH in `codex::build_augmented_path`
/// so the sidecar resolves the same way.
fn resolve_one(name: &str) -> Option<PathBuf> {
    // Reuse the same broad PATH augmentation as the Codex resolver so brew /
    // scoop / etc. installs are found from a GUI launch (minimal inherited PATH).
    // `~/.cameo/bin` is the LAST entry there, so a user install is preferred and
    // the managed copy is the natural fallback in one `which_in` pass.
    let search = crate::codex::build_augmented_path(false);
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(found) = which::which_in(name, Some(&search), cwd).ok().filter(|p| runs_ok(p)) {
        return Some(found);
    }
    // Defensive: the managed binary exists but wasn't on the search path for
    // some reason — try it directly.
    let managed = bin_dir().join(exe(name));
    (managed.is_file() && runs_ok(&managed)).then_some(managed)
}

/// Resolve both tools. Caches only a successful `Some(Tools)` — never the
/// `None` (missing) result — so a user who installs ffmpeg system-wide WHILE
/// Cameo is running is picked up on the next call instead of being stuck at
/// "missing" until restart (§3 negative-cache). The miss path re-probes each
/// call, but it's cheap (a `which` lookup; only spawns `-version` on a hit).
pub fn resolved() -> Option<Tools> {
    if let Some(cached) = RESOLVED.lock().clone() {
        return Some(cached);
    }
    let tools = match (resolve_one("ffmpeg"), resolve_one("ffprobe")) {
        (Some(ffmpeg), Some(ffprobe)) => Some(Tools { ffmpeg, ffprobe }),
        _ => None,
    };
    if let Some(t) = &tools {
        *RESOLVED.lock() = Some(t.clone());
    }
    tools
}

fn invalidate() {
    *RESOLVED.lock() = None;
}

fn version_line(ffmpeg: &Path) -> Option<String> {
    let mut cmd = Command::new(ffmpeg);
    cmd.arg("-version");
    crate::process::hide_console_window(&mut cmd);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let (_status, stdout) = output_with_deadline(cmd, RUN_TIMEOUT)?;
    let text = String::from_utf8_lossy(&stdout);
    text.lines().next().map(|l| l.trim().to_string())
}

/// Current status for the UI. `installing` is reported while a download runs.
pub fn status() -> FfmpegStatus {
    if INSTALLING.load(Ordering::SeqCst) {
        return FfmpegStatus {
            state: "installing".into(),
            ffmpeg_path: None,
            ffprobe_path: None,
            version: None,
            error: None,
        };
    }
    match resolved() {
        Some(t) => FfmpegStatus {
            state: "ready".into(),
            version: version_line(&t.ffmpeg),
            ffmpeg_path: Some(t.ffmpeg.to_string_lossy().into_owned()),
            ffprobe_path: Some(t.ffprobe.to_string_lossy().into_owned()),
            error: None,
        },
        None => FfmpegStatus {
            state: "missing".into(),
            ffmpeg_path: None,
            ffprobe_path: None,
            version: None,
            error: None,
        },
    }
}

// ── ffprobe / poster ─────────────────────────────────────────────────────────

fn parse_fps(rate: &str) -> Option<f64> {
    // ffprobe gives `avg_frame_rate` as "num/den" (e.g. "30000/1001").
    let (n, d) = rate.split_once('/')?;
    let n: f64 = n.trim().parse().ok()?;
    let d: f64 = d.trim().parse().ok()?;
    if d == 0.0 {
        return None;
    }
    let fps = n / d;
    (fps.is_finite() && fps > 0.0).then_some(fps)
}

/// Probe a video for dimensions / duration / fps / audio. `None` if ffprobe is
/// unavailable or the file isn't a parseable video.
pub fn probe_video(path: &Path) -> Option<VideoMeta> {
    let tools = resolved()?;
    let mut cmd = Command::new(&tools.ffprobe);
    cmd.args([
        "-v",
        "error",
        "-show_streams",
        "-show_format",
        "-of",
        "json",
    ])
    .arg(path);
    crate::process::hide_console_window(&mut cmd);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let (status, stdout) = output_with_deadline(cmd, PROBE_TIMEOUT)?;
    if !status.success() {
        return None;
    }
    let json: Value = serde_json::from_slice(&stdout).ok()?;
    let streams = json.get("streams")?.as_array()?;

    let mut meta = VideoMeta::default();
    let mut found_video = false;
    for s in streams {
        match s.get("codec_type").and_then(|v| v.as_str()) {
            Some("video") if !found_video => {
                found_video = true;
                meta.width = s.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                meta.height = s.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                meta.fps = s
                    .get("avg_frame_rate")
                    .and_then(|v| v.as_str())
                    .and_then(parse_fps)
                    .or_else(|| {
                        s.get("r_frame_rate")
                            .and_then(|v| v.as_str())
                            .and_then(parse_fps)
                    });
            }
            Some("audio") => meta.has_audio = true,
            _ => {}
        }
    }
    if !found_video {
        return None;
    }
    meta.duration_ms = json
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .map(|secs| secs * 1000.0);
    Some(meta)
}

/// Extract the first frame to a JPEG poster. Returns false if ffmpeg is missing
/// or the extraction failed (caller treats the video as poster-less).
///
/// Writes to a temp sibling and renames on success only — a timed-out / killed
/// ffmpeg never leaves a half-written poster at `out` (which `extract_video_poster`
/// would then treat as a valid cached poster forever). W2 + §3 resource-leak.
pub fn extract_poster(video: &Path, out: &Path) -> bool {
    let Some(tools) = resolved() else {
        return false;
    };
    let dir = match out.parent() {
        Some(d) => {
            let _ = std::fs::create_dir_all(d);
            d.to_path_buf()
        }
        None => return false,
    };
    // The temp MUST keep a `.jpg` extension: ffmpeg infers the output muxer from
    // the filename, so a `.tmp` suffix makes it fail with "Unable to choose an
    // output format". Use a hidden, randomized basename so it can't collide with
    // the real poster or another concurrent extraction, but end in `.jpg`.
    let tmp = dir.join(format!(".poster-{}.jpg", nanoid::nanoid!(8)));
    let mut cmd = Command::new(&tools.ffmpeg);
    cmd.args(["-y", "-loglevel", "error", "-ss", "0"])
        .arg("-i")
        .arg(video)
        .args(["-frames:v", "1", "-q:v", "3"])
        .arg(&tmp);
    crate::process::hide_console_window(&mut cmd);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let ok = matches!(output_with_deadline(cmd, PROBE_TIMEOUT), Some((s, _)) if s.success())
        && tmp.is_file();
    if ok && std::fs::rename(&tmp, out).is_ok() {
        return true;
    }
    let _ = std::fs::remove_file(&tmp); // clean partial/failed temp
    false
}

/// Extract the frame at `at_seconds` to `out` as PNG (lossless — the agent reads
/// this still as a reference). Returns false if ffmpeg is missing or extraction
/// failed. Same temp-then-rename safety as `extract_poster`. `-ss` before `-i`
/// is a fast, accurate input seek in modern ffmpeg.
pub fn extract_frame_at(video: &Path, out: &Path, at_seconds: f64) -> bool {
    let Some(tools) = resolved() else {
        return false;
    };
    let dir = match out.parent() {
        Some(d) => {
            let _ = std::fs::create_dir_all(d);
            d.to_path_buf()
        }
        None => return false,
    };
    // Keep a `.png` extension so ffmpeg infers the PNG muxer (see extract_poster).
    // Prefix `.overlay-` so a crash between write and rename leaves an orphan the
    // board-open `sweep_overlays` reclaims (it only matches `.overlay-*.png`).
    let tmp = dir.join(format!(".overlay-frameref-tmp-{}.png", nanoid::nanoid!(8)));
    let ss = format!("{:.3}", at_seconds.max(0.0));
    let mut cmd = Command::new(&tools.ffmpeg);
    cmd.args(["-y", "-loglevel", "error", "-ss"])
        .arg(&ss)
        .arg("-i")
        .arg(video)
        .args(["-frames:v", "1"])
        .arg(&tmp);
    crate::process::hide_console_window(&mut cmd);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let ok = matches!(output_with_deadline(cmd, PROBE_TIMEOUT), Some((s, _)) if s.success())
        && tmp.is_file();
    if ok && std::fs::rename(&tmp, out).is_ok() {
        return true;
    }
    let _ = std::fs::remove_file(&tmp);
    false
}

// ── Install (R2 pinned build) ────────────────────────────────────────────────

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Progress {
    file: String,
    downloaded: u64,
    total: u64,
}

fn platform_key() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("mac-arm64"),
        ("macos", "x86_64") => Some("mac-x64"),
        ("windows", "x86_64") => Some("win-x64"),
        _ => None,
    }
}

/// macOS: ad-hoc sign so an unsigned downloaded arm64 binary isn't `killed:9`.
#[cfg(target_os = "macos")]
fn adhoc_sign(path: &Path) {
    let mut cmd = Command::new("codesign");
    cmd.args(["--force", "--sign", "-"]).arg(path);
    crate::process::hide_console_window(&mut cmd);
    match cmd.output() {
        Ok(o) if o.status.success() => {}
        Ok(o) => tracing::warn!(
            module = "tools",
            "codesign ffmpeg failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ),
        Err(e) => tracing::warn!(module = "tools", "codesign spawn failed: {e}"),
    }
}

/// Stream one binary to a temp file while hashing, verify the blake3 pin, make
/// it executable, and return the VERIFIED TEMP path (NOT yet promoted to its
/// final name). The caller promotes ffmpeg + ffprobe together only after BOTH
/// verify, so a mid-set failure never leaves a half install (W5).
async fn download_verified(
    app: &AppHandle,
    name: &str,
    entry: &Value,
) -> Result<PathBuf, String> {
    let url = entry
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{name}: manifest missing url"))?;
    let want_hash = entry
        .get("blake3")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{name}: manifest missing blake3"))?
        .to_lowercase();
    let declared_total = entry.get("size").and_then(|v| v.as_u64()).unwrap_or(0);

    let mut resp = crate::net::client()
        .get(url)
        // The shared product client has a short timeout for JSON/API calls.
        // Windows ffmpeg builds are ~220 MB each, so the managed tool download
        // needs its own end-to-end budget while still using the proxied client.
        .timeout(DOWNLOAD_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("{name}: download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("{name}: download HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(declared_total);

    let dir = bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{name}: mkdir bin: {e}"))?;
    let tmp_path = dir.join(format!(".{}.{}.tmp", exe(name), nanoid::nanoid!(6)));

    let mut file = std::fs::File::create(&tmp_path)
        .map_err(|e| format!("{name}: create temp: {e}"))?;
    let mut hasher = blake3::Hasher::new();
    let mut downloaded: u64 = 0;
    let mut last_emit = Instant::now();

    loop {
        let chunk = match resp.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(format!("{name}: stream error: {e}"));
            }
        };
        hasher.update(&chunk);
        if let Err(e) = file.write_all(&chunk) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!("{name}: write error: {e}"));
        }
        downloaded += chunk.len() as u64;
        // Throttle progress emits (~10/s) so the IPC channel isn't flooded.
        if last_emit.elapsed() >= Duration::from_millis(100) {
            last_emit = Instant::now();
            let _ = app.emit(
                "ffmpeg:progress",
                Progress {
                    file: name.to_string(),
                    downloaded,
                    total,
                },
            );
        }
    }
    drop(file);

    let got = hasher.finalize().to_hex().to_string();
    if got != want_hash {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(format!(
            "{name}: checksum mismatch (expected {want_hash}, got {got})"
        ));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755));
    }

    let _ = app.emit(
        "ffmpeg:progress",
        Progress {
            file: name.to_string(),
            downloaded: total.max(downloaded),
            total,
        },
    );
    // Verified + executable, still under its temp name. Promotion happens in
    // install_inner once the whole set is verified (W5).
    Ok(tmp_path)
}

/// Result of promoting one verified temp binary: where it landed, and (if a
/// binary already existed there) the `.bak` it was moved aside to, so the whole
/// install can roll back to the prior set if a later promote fails.
struct Promoted {
    final_path: PathBuf,
    backup: Option<PathBuf>,
}

/// Move a verified temp binary into its final managed name, then macOS ad-hoc
/// sign. Any existing binary at the destination is moved aside to a `.bak`
/// FIRST (not deleted) so the caller can restore it on rollback — without that,
/// a failed later promote would leave the prior set destroyed (the very
/// half-install state this flow avoids). Windows `rename` also can't overwrite,
/// so moving the old one aside is required there regardless.
fn promote(name: &str, tmp_path: &Path) -> Result<Promoted, String> {
    let final_path = bin_dir().join(exe(name));
    let mut backup = None;
    if final_path.exists() {
        let bak = final_path.with_extension(format!("bak.{}", nanoid::nanoid!(6)));
        std::fs::rename(&final_path, &bak)
            .map_err(|e| format!("{name}: back up existing failed: {e}"))?;
        backup = Some(bak);
    }
    if let Err(e) = std::fs::rename(tmp_path, &final_path) {
        // Restore the binary we moved aside, then clean the temp.
        if let Some(bak) = &backup {
            let _ = std::fs::rename(bak, &final_path);
        }
        let _ = std::fs::remove_file(tmp_path);
        return Err(format!("{name}: install move failed: {e}"));
    }
    #[cfg(target_os = "macos")]
    adhoc_sign(&final_path);
    Ok(Promoted { final_path, backup })
}

/// Remove orphaned `.<name>.<nanoid>.tmp` files left by a crash/hard-kill mid
/// download (every error path already cleans its own temp; this sweeps the
/// crash case, mirroring assets::sweep_overlays). §3 startup-clean. Public so it
/// can also run once at boot. NOTE: it deliberately does NOT touch `*.bak.*`
/// promotion backups — a crash between "move old aside" and "rename new in"
/// could leave the bak as the only surviving copy, so a stale bak is left in
/// place (harmless: resolve_one only looks for the exact `ffmpeg`/`ffprobe`
/// names, never a `.bak`).
pub fn sweep_bin_temps() {
    let Ok(rd) = std::fs::read_dir(bin_dir()) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') && name.ends_with(".tmp") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Resets the `INSTALLING` flag on drop — so it clears on EVERY exit, including
/// an unexpected panic in the install path (an early `store(false)` would be
/// skipped by an unwind and wedge `status()` at "installing" forever). §3 RAII.
struct InstallGuard;
impl Drop for InstallGuard {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::SeqCst);
    }
}

/// Error returned when an install is already running — not a real failure. The
/// frontend (SettingsModal) recognizes this string and doesn't show it as an
/// error (a concurrent install is already in flight). §3.
const ALREADY_IN_PROGRESS: &str = "install already in progress";

/// Download + verify ffmpeg AND ffprobe from the R2 manifest. Emits
/// `ffmpeg:progress` during, then `ffmpeg:done` / `ffmpeg:failed`.
pub async fn install(app: AppHandle) -> Result<(), String> {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err(ALREADY_IN_PROGRESS.into());
    }
    let guard = InstallGuard; // resets INSTALLING on every exit, panic included
    let result = install_inner(&app).await;
    if result.is_ok() {
        invalidate();
    }
    // Clear the installing flag BEFORE emitting the terminal event: SettingsModal
    // refreshes via `tool_status()` on `ffmpeg:done`/`ffmpeg:failed`, and would
    // otherwise read a stale "installing" state and never leave it. The guard
    // stays as a panic-only backstop.
    drop(guard);
    match &result {
        Ok(()) => {
            let _ = app.emit("ffmpeg:done", ());
        }
        Err(e) => {
            tracing::warn!(module = "tools", "ffmpeg install failed: {e}");
            let _ = app.emit("ffmpeg:failed", e.clone());
        }
    }
    result
}

async fn install_inner(app: &AppHandle) -> Result<(), String> {
    sweep_bin_temps(); // clear any orphaned temps from a prior crash

    let key = platform_key().ok_or_else(|| {
        format!(
            "no ffmpeg build for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;

    let resp = crate::net::client()
        .get(MANIFEST_URL)
        .send()
        .await
        .map_err(|e| format!("manifest fetch failed: {e}"))?;
    // Check status BEFORE parsing (mirrors download_verified): a 404 means the
    // managed-download channel isn't published yet, and its text/plain body
    // would otherwise surface as a cryptic "manifest parse failed" JSON error.
    // The user's own PATH ffmpeg still works (detect-first) — say so.
    if resp.status().as_u16() == 404 {
        return Err("ffmpeg download isn't available yet — install ffmpeg yourself (e.g. `brew install ffmpeg`) and Cameo will detect it automatically".into());
    }
    if !resp.status().is_success() {
        return Err(format!("manifest fetch HTTP {}", resp.status()));
    }
    let manifest: Value = resp
        .json()
        .await
        .map_err(|e| format!("manifest parse failed: {e}"))?;

    let build = manifest
        .get("builds")
        .and_then(|b| b.get(key))
        .ok_or_else(|| format!("manifest has no build for {key}"))?;

    // Phase 1: download + verify BOTH to temp. A failure here promotes nothing,
    // so the managed dir never holds a half set (W5).
    let mut staged: Vec<(&str, PathBuf)> = Vec::new();
    for name in ["ffmpeg", "ffprobe"] {
        let entry = match build.get(name) {
            Some(e) => e,
            None => {
                for (_, t) in &staged {
                    let _ = std::fs::remove_file(t);
                }
                return Err(format!("manifest build {key} missing {name}"));
            }
        };
        match download_verified(app, name, entry).await {
            Ok(tmp) => staged.push((name, tmp)),
            Err(e) => {
                for (_, t) in &staged {
                    let _ = std::fs::remove_file(t);
                }
                return Err(e);
            }
        }
    }

    // Phase 2: promote both. If one fails partway, ROLL BACK to the prior set —
    // remove the ones we just promoted and restore their backups — so a failed
    // update never destroys a working install ([P2] preserve old set).
    let mut promoted: Vec<Promoted> = Vec::new();
    for (name, tmp) in &staged {
        match promote(name, tmp) {
            Ok(p) => promoted.push(p),
            Err(e) => {
                for done in &promoted {
                    let _ = std::fs::remove_file(&done.final_path);
                    if let Some(bak) = &done.backup {
                        let _ = std::fs::rename(bak, &done.final_path);
                    }
                }
                for (_, t) in &staged {
                    let _ = std::fs::remove_file(t);
                }
                return Err(e);
            }
        }
    }
    // Verify the freshly-promoted pair actually runs on this arch — BEFORE
    // discarding the backups. A post-promote failure (e.g. codesign/exec error)
    // must not leave broken binaries installed with the prior working pair
    // already deleted ([P2] roll back before discarding backups).
    invalidate();
    if resolved().is_some() {
        // Verified good — now it's safe to drop the backups of replaced binaries.
        for done in &promoted {
            if let Some(bak) = &done.backup {
                let _ = std::fs::remove_file(bak);
            }
        }
        tracing::info!(module = "tools", "ffmpeg installed into managed bin dir");
        Ok(())
    } else {
        // The new binaries don't run — restore the previous working pair.
        for done in &promoted {
            let _ = std::fs::remove_file(&done.final_path);
            if let Some(bak) = &done.backup {
                let _ = std::fs::rename(bak, &done.final_path);
            }
        }
        invalidate(); // re-resolve against the restored set next call
        Err("installed binaries did not verify (not executable?)".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_parses_ratios() {
        assert_eq!(parse_fps("30/1"), Some(30.0));
        assert!((parse_fps("30000/1001").unwrap() - 29.97).abs() < 0.01);
        assert_eq!(parse_fps("0/0"), None);
        assert_eq!(parse_fps("garbage"), None);
    }

    #[test]
    fn platform_key_is_known_for_supported_targets() {
        // At least confirm the call doesn't panic on the host.
        let _ = platform_key();
    }

    /// Regression for the pipe-buffer deadlock: a child that writes far more
    /// than the OS pipe buffer (~64 KiB) to stdout must still complete, because
    /// stdout is drained concurrently rather than after exit. Before the fix
    /// this would hang until the deadline killed a healthy process.
    #[test]
    #[cfg(unix)]
    fn drains_stdout_larger_than_pipe_buffer() {
        let big = 1_000_000; // ~1 MB, well over the pipe buffer
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(format!("yes x | head -c {big}"))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let (status, stdout) =
            output_with_deadline(cmd, Duration::from_secs(10)).expect("should not time out");
        assert!(status.success());
        assert_eq!(stdout.len(), big);
    }

    #[test]
    #[cfg(unix)]
    fn deadline_kills_a_hung_child() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("sleep 30")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        let start = Instant::now();
        let out = output_with_deadline(cmd, Duration::from_millis(300));
        assert!(out.is_none());
        assert!(start.elapsed() < Duration::from_secs(5)); // killed promptly, not after 30s
    }
}
