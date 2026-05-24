//! `cameo://<boardId>/<rel-path>` — serves image bytes from a Board folder.
//!
//! Path canonicalization + traversal guard ported from Riff's `riff://` scheme:
//! reject `..`/absolute components, then verify the canonical path stays inside
//! the Board folder.

use crate::board::BoardRegistry;
use std::path::{Component, PathBuf};
use std::sync::Arc;
use tauri::http::{header, Request, Response, StatusCode};
use tauri::{Manager, UriSchemeContext};

pub fn handle_cameo_uri<R: tauri::Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let uri = request.uri();

    let board_id = match uri.host() {
        Some(h) if !h.is_empty() => h.to_string(),
        _ => return error_response(StatusCode::BAD_REQUEST, "missing boardId in cameo:// URL"),
    };

    let app = ctx.app_handle();
    let registry = app.state::<Arc<BoardRegistry>>();
    let folder = match registry.folder(&board_id) {
        Some(f) => f,
        None => return error_response(StatusCode::NOT_FOUND, "unknown board"),
    };

    let rel_raw = uri.path().trim_start_matches('/');
    let rel = urlencoding::decode(rel_raw)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| rel_raw.to_string());

    let rel_path = PathBuf::from(&rel);
    for comp in rel_path.components() {
        if !matches!(comp, Component::Normal(_)) {
            return error_response(StatusCode::FORBIDDEN, "path traversal blocked");
        }
    }

    let abs = folder.join(&rel_path);
    let canonical = match std::fs::canonicalize(&abs) {
        Ok(p) => p,
        Err(_) => return error_response(StatusCode::NOT_FOUND, "file not found"),
    };
    let base_canonical = match std::fs::canonicalize(&folder) {
        Ok(p) => p,
        Err(_) => return error_response(StatusCode::NOT_FOUND, "board folder gone"),
    };
    if !canonical.starts_with(&base_canonical) {
        return error_response(StatusCode::FORBIDDEN, "escape attempt blocked");
    }

    let bytes = match std::fs::read(&canonical) {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let mime = mime_guess::from_path(&canonical)
        .first_or_octet_stream()
        .to_string();

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        // The webview origin (http://localhost:1420 in dev, tauri://localhost in
        // prod) fetches this cross-origin for textures.
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-store")
        .body(bytes)
        .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "build response"))
}

fn error_response(code: StatusCode, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(code)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("Access-Control-Allow-Origin", "*")
        .body(msg.as_bytes().to_vec())
        .unwrap()
}
