//! Managed ffmpeg/ffprobe: detect → (silent download) → verify → probe/poster.
//!
//! Resolution order (decision E1 — the user's own install wins):
//!   1. `~/.cameo/bin/` (a build Cameo downloaded earlier)
//!   2. system PATH + the same broad augmentation Codex uses (brew, etc.)
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
const MANIFEST_URL: &str = "https://r.cameo.ink/tools/ffmpeg/manifest.json";

/// One install runs at a time (the UI button + an auto-trigger could both fire).
static INSTALLING: AtomicBool = AtomicBool::new(false);
/// Cached resolution; invalidated after a successful install.
static RESOLVED: Mutex<Option<Option<Tools>>> = Mutex::new(None);

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

/// Find one tool: managed dir first (E1: still prefer a working system build via
/// the augmented PATH if the managed one is somehow broken), then PATH.
fn resolve_one(name: &str) -> Option<PathBuf> {
    let managed = bin_dir().join(exe(name));
    if managed.is_file() && runs_ok(&managed) {
        return Some(managed);
    }
    // Reuse the same broad PATH augmentation as the Codex resolver so brew /
    // scoop / etc. installs are found from a GUI launch (minimal inherited PATH).
    let search = crate::codex::build_augmented_path(false);
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    which::which_in(name, Some(&search), cwd)
        .ok()
        .filter(|p| runs_ok(p))
}

/// Resolve both tools, caching the result. `None` means at least one is missing.
pub fn resolved() -> Option<Tools> {
    if let Some(cached) = RESOLVED.lock().as_ref() {
        return cached.clone();
    }
    let tools = match (resolve_one("ffmpeg"), resolve_one("ffprobe")) {
        (Some(ffmpeg), Some(ffprobe)) => Some(Tools { ffmpeg, ffprobe }),
        _ => None,
    };
    *RESOLVED.lock() = Some(tools.clone());
    tools
}

fn invalidate() {
    *RESOLVED.lock() = None;
}

fn version_line(ffmpeg: &Path) -> Option<String> {
    let mut cmd = Command::new(ffmpeg);
    cmd.arg("-version");
    crate::process::hide_console_window(&mut cmd);
    let out = cmd.output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
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
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let json: Value = serde_json::from_slice(&out.stdout).ok()?;
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
pub fn extract_poster(video: &Path, out: &Path) -> bool {
    let Some(tools) = resolved() else {
        return false;
    };
    if let Some(dir) = out.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut cmd = Command::new(&tools.ffmpeg);
    cmd.args(["-y", "-loglevel", "error", "-ss", "0"])
        .arg("-i")
        .arg(video)
        .args(["-frames:v", "1", "-q:v", "3"])
        .arg(out);
    crate::process::hide_console_window(&mut cmd);
    match cmd.output() {
        Ok(o) if o.status.success() && out.is_file() => true,
        _ => false,
    }
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

/// Stream one binary to a temp file while hashing, verify the blake3 pin, then
/// atomically move it into the managed bin dir (+ chmod/sign).
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
        .send()
        .await
        .map_err(|e| format!("{name}: download failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("{name}: download HTTP {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(declared_total);

    let dir = bin_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("{name}: mkdir bin: {e}"))?;
    let final_path = dir.join(exe(name));
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
    // Atomic swap into place only after the bytes are verified + executable.
    std::fs::rename(&tmp_path, &final_path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        format!("{name}: install move failed: {e}")
    })?;
    #[cfg(target_os = "macos")]
    adhoc_sign(&final_path);

    let _ = app.emit(
        "ffmpeg:progress",
        Progress {
            file: name.to_string(),
            downloaded: total.max(downloaded),
            total,
        },
    );
    Ok(final_path)
}

/// Download + verify ffmpeg AND ffprobe from the R2 manifest. Emits
/// `ffmpeg:progress` during, then `ffmpeg:done` / `ffmpeg:failed`.
pub async fn install(app: AppHandle) -> Result<(), String> {
    if INSTALLING.swap(true, Ordering::SeqCst) {
        return Err("install already in progress".into());
    }
    let result = install_inner(&app).await;
    INSTALLING.store(false, Ordering::SeqCst);
    match &result {
        Ok(()) => {
            invalidate();
            let _ = app.emit("ffmpeg:done", ());
        }
        Err(e) => {
            let _ = app.emit("ffmpeg:failed", e.clone());
        }
    }
    result
}

async fn install_inner(app: &AppHandle) -> Result<(), String> {
    let key = platform_key().ok_or_else(|| {
        format!(
            "no ffmpeg build for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;

    let manifest: Value = crate::net::client()
        .get(MANIFEST_URL)
        .send()
        .await
        .map_err(|e| format!("manifest fetch failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("manifest parse failed: {e}"))?;

    let build = manifest
        .get("builds")
        .and_then(|b| b.get(key))
        .ok_or_else(|| format!("manifest has no build for {key}"))?;

    for name in ["ffmpeg", "ffprobe"] {
        let entry = build
            .get(name)
            .ok_or_else(|| format!("manifest build {key} missing {name}"))?;
        download_verified(app, name, entry).await?;
    }

    invalidate();
    // Confirm the freshly-installed pair actually runs on this arch.
    match resolved() {
        Some(_) => {
            tracing::info!(module = "tools", "ffmpeg installed into managed bin dir");
            Ok(())
        }
        None => Err("installed binaries did not verify (not executable?)".into()),
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
}
