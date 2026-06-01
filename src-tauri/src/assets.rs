//! Asset minting + import. Assets are content-addressed (blake3) and immutable;
//! importing the same bytes twice dedups to one Asset (decisions: non-destructive).

use crate::model::{Asset, Origin};
use anyhow::{Context, Result};
use std::io::Cursor;
use std::path::Path;

const IMAGE_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "avif",
];

/// Video containers we treat as time-based media (decision: Codex × ffmpeg).
const VIDEO_EXTS: &[&str] = &["mp4", "webm", "mov", "m4v"];

pub fn is_image_ext(ext: &str) -> bool {
    IMAGE_EXTS.contains(&ext)
}

pub fn is_video_ext(ext: &str) -> bool {
    VIDEO_EXTS.contains(&ext)
}

/// Any media (image or video) Cameo can place on the canvas.
pub fn is_media_ext(ext: &str) -> bool {
    is_image_ext(ext) || is_video_ext(ext)
}

fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default()
}

/// Placeholder square for a video whose real dimensions are unknown (ffmpeg
/// missing / probe failed). Must match the frontend's `asset?.width || 480`
/// fallback in scene.ts so layout and rendered size agree (W3). Also used by
/// backfill to anchor legacy 0×0 placements on their displayed footprint.
pub const NOMINAL_VIDEO_DIM: u32 = 480;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn hash_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

pub fn hash_file_hex(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(hash_hex(&bytes))
}

/// Read width/height from encoded bytes without a full decode.
fn dims_from_bytes(bytes: &[u8]) -> (u32, u32) {
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()
        .and_then(|r| r.into_dimensions().ok())
        .unwrap_or((0, 0))
}

/// Mint an Asset for a file that already lives in the Board folder.
///
/// This is the single funnel for every importer (drag/drop, agent output,
/// clipboard, reconcile), so media-type handling added here applies everywhere
/// (pit of success). Video files are enriched via ffprobe (dims / duration /
/// fps / audio) and get a poster frame extracted via ffmpeg; if ffmpeg isn't
/// installed yet, the video still mints (poster-less, zero dims) and the UI
/// offers to install it.
pub fn mint_asset(folder: &Path, rel_path: &str, origin: Origin) -> Result<Asset> {
    let abs = folder.join(rel_path);
    let mime = mime_guess::from_path(&abs)
        .first_or_octet_stream()
        .to_string();

    if mime.starts_with("video/") || is_video_ext(&ext_of(&abs)) {
        let id = hash_file_hex(&abs)?;
        let meta = crate::tools::ffmpeg::probe_video(&abs);
        // When ffmpeg is missing (or ffprobe gave 0×0), fall back to a nominal
        // square that matches the frontend's `asset?.width || 480` placeholder
        // size (scene.ts) — otherwise Rust's footprint/flow_layout treats the
        // tile as zero-size and stacks multiple poster-less videos on the same
        // spot while the canvas draws 480px blocks that overlap (W3). A later
        // backfill re-mints with the true dimensions.
        let (width, height) = meta
            .as_ref()
            .map(|m| (m.width, m.height))
            .filter(|&(w, h)| w > 0 && h > 0)
            .unwrap_or((NOMINAL_VIDEO_DIM, NOMINAL_VIDEO_DIM));
        let poster_path = extract_video_poster(folder, &abs, &id);
        return Ok(Asset {
            id,
            path: rel_path.to_string(),
            width,
            height,
            mime,
            created_at: now_ms(),
            origin,
            duration_ms: meta.as_ref().and_then(|m| m.duration_ms),
            fps: meta.as_ref().and_then(|m| m.fps),
            has_audio: meta.as_ref().map(|m| m.has_audio),
            poster_path,
        });
    }

    let bytes = std::fs::read(&abs).with_context(|| format!("read {}", abs.display()))?;
    let (width, height) = dims_from_bytes(&bytes);
    Ok(Asset {
        id: hash_hex(&bytes),
        path: rel_path.to_string(),
        width,
        height,
        mime,
        created_at: now_ms(),
        origin,
        duration_ms: None,
        fps: None,
        has_audio: None,
        poster_path: None,
    })
}

/// Extract (or reuse) the first-frame poster for a video into the Board sidecar
/// (`.cameo/posters/<hash>.jpg`). Returns the board-relative path, or `None`
/// when ffmpeg is unavailable / extraction failed. Content-addressed by the
/// video's hash so re-minting identical bytes reuses one poster.
fn extract_video_poster(folder: &Path, video_abs: &Path, id: &str) -> Option<String> {
    let rel = format!(".cameo/posters/{id}.jpg");
    let out = folder.join(&rel);
    if out.is_file() {
        return Some(rel);
    }
    if crate::tools::ffmpeg::extract_poster(video_abs, &out) {
        Some(rel)
    } else {
        None
    }
}

/// Pick a non-colliding filename `<stem>.<ext>` (or `<stem>-N.<ext>`). Used for
/// imports that keep the user's original filename.
fn unique_name(folder: &Path, stem: &str, ext: &str) -> String {
    let stem = if stem.is_empty() { "image" } else { stem };
    let mut name = format!("{stem}.{ext}");
    let mut i = 1;
    while folder.join(&name).exists() {
        name = format!("{stem}-{i}.{ext}");
        i += 1;
    }
    name
}

/// `<stem>-<YYYYMMDD-HHMMSS>.<ext>`, unique in the folder (a `-N` suffix breaks
/// same-second collisions). Naming protocol for app-minted files: human-readable,
/// chronologically sortable, and the stem records provenance (gen / crop / paste).
fn timestamped_name(folder: &Path, stem: &str, ext: &str) -> String {
    let stem = if stem.is_empty() { "image" } else { stem };
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let mut name = format!("{stem}-{ts}.{ext}");
    let mut i = 1;
    while folder.join(&name).exists() {
        name = format!("{stem}-{ts}-{i}.{ext}");
        i += 1;
    }
    name
}

/// Destination extension for a copied media file: keep the source extension if
/// it's a known image or video, else fall back to png (images dominate; a bogus
/// ext shouldn't poison the filename).
fn media_ext_of(src: &Path) -> String {
    src.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .filter(|e| is_media_ext(e))
        .unwrap_or_else(|| "png".to_string())
}

/// Copy a user-provided external file (drag/drop, picker) into the Board folder,
/// **preserving its original filename**, and mint an `imported` Asset. If
/// identical bytes are already tracked, reuse that Asset (no copy).
pub fn import_external(folder: &Path, src: &Path, existing: &[Asset]) -> Result<Asset> {
    let bytes = std::fs::read(src).with_context(|| format!("read src {}", src.display()))?;
    let id = hash_hex(&bytes);
    if let Some(a) = existing.iter().find(|a| a.id == id) {
        return Ok(a.clone());
    }
    let ext = media_ext_of(src);
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let rel = unique_name(folder, stem, &ext);
    std::fs::write(folder.join(&rel), &bytes)
        .with_context(|| format!("write {}", folder.join(&rel).display()))?;
    mint_asset(folder, &rel, Origin::Imported)
}

/// Copy a Codex-produced file (imageGeneration `savedPath`) into the Board folder
/// under our naming (`gen-<timestamp>`) — NOT Codex's opaque `ig_<hash>` name —
/// and mint a `generated` Asset. Dedups by content like the other importers.
pub fn import_generated_file(folder: &Path, src: &Path, existing: &[Asset]) -> Result<Asset> {
    let bytes = std::fs::read(src).with_context(|| format!("read src {}", src.display()))?;
    let id = hash_hex(&bytes);
    if let Some(a) = existing.iter().find(|a| a.id == id) {
        return Ok(a.clone());
    }
    let ext = media_ext_of(src);
    let rel = timestamped_name(folder, "gen", &ext);
    std::fs::write(folder.join(&rel), &bytes)
        .with_context(|| format!("write {}", folder.join(&rel).display()))?;
    mint_asset(folder, &rel, Origin::Generated)
}

/// Write raw image bytes (clipboard paste, crop bake, base64 generation) into the
/// Board folder under the `<stem>-<timestamp>` naming protocol, dedup, and mint
/// with the given `origin`.
pub fn import_bytes(
    folder: &Path,
    bytes: &[u8],
    ext: &str,
    stem: &str,
    origin: Origin,
    existing: &[Asset],
) -> Result<Asset> {
    let id = hash_hex(bytes);
    if let Some(a) = existing.iter().find(|a| a.id == id) {
        return Ok(a.clone());
    }
    let ext = if is_image_ext(ext) { ext } else { "png" };
    let rel = timestamped_name(folder, stem, ext);
    std::fs::write(folder.join(&rel), bytes)
        .with_context(|| format!("write {}", folder.join(&rel).display()))?;
    mint_asset(folder, &rel, origin)
}

/// Remove stale dispatch overlay temps (`.overlay-*.png`) from the Board root.
/// They're only needed during an in-flight turn (Codex reads them mid-turn); any
/// left behind are junk, since nothing else cleans them up. Called on Board open.
pub fn sweep_overlays(folder: &Path) {
    let Ok(rd) = std::fs::read_dir(folder) else { return };
    for entry in rd.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(".overlay-") && name.ends_with(".png") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Top-level media files (images + videos) in the folder (skips hidden /
/// `.cameo`). Relative names. Drives folder↔board reconciliation, so videos
/// dropped into the folder appear on the canvas just like images.
pub fn scan_images(folder: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(folder) {
        for entry in rd.flatten() {
            let p = entry.path();
            if !p.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let is_media = p
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| is_media_ext(&x.to_lowercase()))
                .unwrap_or(false);
            if is_media {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out
}
