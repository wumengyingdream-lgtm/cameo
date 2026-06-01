//! Cameo image protocol handler. Serves image bytes from a Board folder.
//!
//! URL shape is platform-dependent: WebKit-style webviews request
//! `cameo://localhost/<boardId>/<rel-path>`, while WebView2 requests
//! `http://cameo.localhost/<boardId>/<rel-path>`.
//!
//! Path canonicalization + traversal guard ported from Riff's `riff://` scheme:
//! reject `..`/absolute components, then verify the canonical path stays inside
//! the Board folder.

use crate::board::BoardRegistry;
use std::path::{Component, PathBuf};
use std::sync::Arc;
use tauri::http::{header, Request, Response, StatusCode, Uri};
use tauri::{Manager, UriSchemeContext};

fn parse_cameo_uri(uri: &Uri) -> Result<(String, String), (StatusCode, &'static str)> {
    let host = uri.host().unwrap_or_default();
    let path = uri.path().trim_start_matches('/');

    // Tauri/WebView2 represents custom protocols as `http://<scheme>.localhost/...`
    // on Windows. Keep board routing in the path and only support host-as-board
    // for legacy `cameo://<boardId>/<rel-path>` URLs used before this fix.
    if !host.is_empty() && host != "localhost" && !host.ends_with(".localhost") {
        if path.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                "missing image path in cameo:// URL",
            ));
        }
        return Ok((host.to_string(), path.to_string()));
    }

    let Some((board_raw, rel_raw)) = path.split_once('/') else {
        return Err((StatusCode::BAD_REQUEST, "missing boardId in cameo:// URL"));
    };
    if board_raw.is_empty() || rel_raw.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "missing boardId or image path in cameo:// URL",
        ));
    }
    let board_id = urlencoding::decode(board_raw)
        .map(|c| c.into_owned())
        .unwrap_or_else(|_| board_raw.to_string());
    Ok((board_id, rel_raw.to_string()))
}

pub fn handle_cameo_uri<R: tauri::Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let uri = request.uri();

    let (board_id, rel_raw) = match parse_cameo_uri(uri) {
        Ok(parsed) => parsed,
        Err((code, msg)) => return error_response(code, msg),
    };

    let app = ctx.app_handle();
    let registry = app.state::<Arc<BoardRegistry>>();
    let folder = match registry.folder(&board_id) {
        Some(f) => f,
        None => return error_response(StatusCode::NOT_FOUND, "unknown board"),
    };

    let rel = urlencoding::decode(&rel_raw)
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

    // Size from metadata — never read the whole file just to learn its length.
    let total = match std::fs::metadata(&canonical) {
        Ok(m) => m.len(),
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let mime = mime_guess::from_path(&canonical)
        .first_or_octet_stream()
        .to_string();

    // `<video>` seek (frame scrubbing) requires byte-range support — without a
    // 206 the element refuses to seek. The element always sends a `Range`, so
    // this branch is the ONLY path videos take: serve just the requested slice
    // STREAMED from disk (seek + bounded read), never the whole multi-GB file.
    // A single response is capped at MAX_RANGE_CHUNK; `bytes=0-` (whole file)
    // therefore returns the first chunk as 206 and the player re-requests the
    // rest — standard range behavior, bounded memory.
    if let Some(range_header) = request
        .headers()
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
    {
        match parse_byte_range(range_header, total) {
            Some((start0, end0, is_suffix)) => {
                // Cap the response to MAX_RANGE_CHUNK so peak memory stays bounded
                // regardless of file size. For a suffix request (`bytes=-N`, "give
                // me the LAST N bytes" — WebKit uses these to read trailing mp4
                // metadata), keep the END and move the start forward; otherwise
                // keep the START and lower the end. Either way the client reads
                // Content-Range and re-requests the rest.
                let (start, end) = if is_suffix {
                    (end0.saturating_sub(MAX_RANGE_CHUNK - 1).max(start0), end0)
                } else {
                    (start0, end0.min(start0.saturating_add(MAX_RANGE_CHUNK - 1)))
                };
                let len = end - start + 1;
                let slice = match read_file_range(&canonical, start, len) {
                    Ok(b) => b,
                    Err(e) => {
                        return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
                    }
                };
                return Response::builder()
                    .status(StatusCode::PARTIAL_CONTENT)
                    .header(header::CONTENT_TYPE, mime)
                    .header(header::ACCEPT_RANGES, "bytes")
                    .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"))
                    .header("Access-Control-Allow-Origin", "*")
                    .header("Cache-Control", "no-store")
                    .body(slice)
                    .unwrap_or_else(|_| {
                        error_response(StatusCode::INTERNAL_SERVER_ERROR, "build response")
                    });
            }
            None => {
                // A Range header was present but unsatisfiable/malformed. Do NOT
                // fall through to a full read (that's the multi-GB OOM). Reply
                // 416 with the resource size, per RFC 7233.
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{total}"))
                    .header(header::ACCEPT_RANGES, "bytes")
                    .header("Access-Control-Allow-Origin", "*")
                    .body(Vec::new())
                    .unwrap_or_else(|_| {
                        error_response(StatusCode::INTERNAL_SERVER_ERROR, "build response")
                    });
            }
        }
    }

    // No Range header → full body. This path is images (small; loaded via
    // fetch→blob→createImageBitmap, which needs the whole 200). Videos never
    // arrive here. A defensive cap rejects an implausibly large full read.
    if total > MAX_FULL_READ {
        return error_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "resource too large for a non-range request",
        );
    }
    let bytes = match std::fs::read(&canonical) {
        Ok(b) => b,
        Err(e) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        // Advertise range support so `<video>` knows it can seek on a re-request.
        .header(header::ACCEPT_RANGES, "bytes")
        // The webview origin (http://localhost:1420 in dev, tauri://localhost in
        // prod) fetches this cross-origin for textures.
        .header("Access-Control-Allow-Origin", "*")
        .header("Cache-Control", "no-store")
        .body(bytes)
        .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "build response"))
}

/// Max bytes returned in one 206 response. `<video>` requesting `bytes=0-` gets
/// the first chunk and re-requests the rest, so playback works while peak
/// allocation stays bounded regardless of file size.
const MAX_RANGE_CHUNK: u64 = 16 * 1024 * 1024; // 16 MiB
/// Cap for a non-range full read (images). Generous for any real image; guards
/// against a pathological full read of a huge file.
const MAX_FULL_READ: u64 = 256 * 1024 * 1024; // 256 MiB

/// Read `[start, start+len)` from a file without buffering the whole thing.
fn read_file_range(path: &std::path::Path, start: u64, len: u64) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; len as usize];
    f.read_exact(&mut buf)?; // len ≤ total-start (end clamped to total-1), so no EOF
    Ok(buf)
}

/// Parse a single `bytes=start-end` range against the resource size. Returns
/// `(start, end, is_suffix)` — inclusive, clamped in-bounds — or `None` when the
/// header is malformed / unsatisfiable (caller replies 416). `is_suffix` is true
/// for a `bytes=-N` request ("last N bytes"), so the caller can cap it without
/// inverting its meaning. Multi-range (`bytes=0-9,20-29`) is intentionally
/// unsupported — `<video>` never sends it.
fn parse_byte_range(header: &str, total: u64) -> Option<(u64, u64, bool)> {
    if total == 0 {
        return None;
    }
    let spec = header.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (start_s, end_s) = spec.split_once('-')?;
    let (start, end, is_suffix) = match (start_s.trim(), end_s.trim()) {
        // Suffix range: last N bytes (`bytes=-500`).
        ("", e) => {
            let n: u64 = e.parse().ok()?;
            if n == 0 {
                return None;
            }
            (total.saturating_sub(n), total - 1, true)
        }
        // Open-ended: from start to EOF (`bytes=500-`).
        (s, "") => (s.parse().ok()?, total - 1, false),
        // Closed range (`bytes=0-1023`).
        (s, e) => (s.parse().ok()?, e.parse::<u64>().ok()?.min(total - 1), false),
    };
    if start > end || start >= total {
        return None;
    }
    Some((start, end, is_suffix))
}

fn error_response(code: StatusCode, msg: &str) -> Response<Vec<u8>> {
    Response::builder()
        .status(code)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header("Access-Control-Allow-Origin", "*")
        .body(msg.as_bytes().to_vec())
        .unwrap()
}

// ── cmnet:// — proxied remote fetch (gallery images) ─────────────────────────
//
// WebView `<img src=https://…>` loads bypass the app proxy. Routing them through
// this scheme makes the proxied `net::client` fetch the bytes, so gallery
// thumbnails/detail images honor Settings → Proxy like every other egress.
// URL shape: `cmnet://localhost/<percent-encoded-https-url>` (WebKit) /
// `http://cmnet.localhost/<…>` (WebView2).

/// Decode the wrapped remote URL from a cmnet request path.
fn parse_cmnet_uri(uri: &Uri) -> Option<String> {
    let path = uri.path().trim_start_matches('/');
    if path.is_empty() {
        return None;
    }
    urlencoding::decode(path).ok().map(|c| c.into_owned())
}

/// True for a host that must never be reachable through the `cmnet://` proxy —
/// loopback / private / link-local / unique-local / unspecified, in v4 or v6
/// (including v4-mapped v6). Centralized so the initial-URL check and the
/// per-redirect-hop check (net.rs) agree exactly.
pub fn is_blocked_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            // A v4-mapped address (::ffff:127.0.0.1) must be judged by its v4
            // form, or loopback/private literals slip through as "v6".
            if let Some(v4) = v6.to_ipv4_mapped() {
                return v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_unspecified();
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || is_unique_local_v6(v6)        // fc00::/7
                || is_unicast_link_local_v6(v6)  // fe80::/10
        }
    }
}

/// `fc00::/7` (unique-local). `Ipv6Addr::is_unique_local` is unstable, so check
/// the high 7 bits directly.
fn is_unique_local_v6(v6: std::net::Ipv6Addr) -> bool {
    (v6.octets()[0] & 0xfe) == 0xfc
}

/// `fe80::/10` (unicast link-local). `is_unicast_link_local` is unstable too.
fn is_unicast_link_local_v6(v6: std::net::Ipv6Addr) -> bool {
    let o = v6.octets();
    o[0] == 0xfe && (o[1] & 0xc0) == 0x80
}

/// SSRF guard: only public https URLs. Blocks loopback / link-local / private
/// ranges so this proxy can't be turned into a probe of the user's intranet.
pub fn is_allowed_remote(url: &str) -> bool {
    let Ok(parsed) = reqwest::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let h = host.to_lowercase();
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".local") {
        return false;
    }
    // `host_str()` returns IPv6 literals WITH brackets ("[::1]"); strip them so
    // the parse succeeds and the address is actually evaluated (else it falls
    // through as "not an IP" → allowed — the bracket-bypass).
    let h_ip = h.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(&h);
    if let Ok(ip) = h_ip.parse::<std::net::IpAddr>() {
        if is_blocked_ip(ip) {
            return false;
        }
    }
    true
}

pub fn handle_cmnet_uri<R: tauri::Runtime>(
    _ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let Some(url) = parse_cmnet_uri(request.uri()) else {
        responder.respond(error_response(StatusCode::BAD_REQUEST, "missing url in cmnet:// request"));
        return;
    };
    if !is_allowed_remote(&url) {
        responder.respond(error_response(StatusCode::FORBIDDEN, "only public https URLs allowed"));
        return;
    }
    tauri::async_runtime::spawn(async move {
        let resp = match crate::net::client().get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                responder.respond(error_response(StatusCode::BAD_GATEWAY, &format!("fetch failed: {e}")));
                return;
            }
        };
        let status = resp.status();
        let mime = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = match resp.bytes().await {
            Ok(b) => b.to_vec(),
            Err(e) => {
                responder.respond(error_response(StatusCode::BAD_GATEWAY, &format!("read failed: {e}")));
                return;
            }
        };
        let out = Response::builder()
            .status(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK))
            .header(header::CONTENT_TYPE, mime)
            .header("Access-Control-Allow-Origin", "*")
            .header(header::CACHE_CONTROL, "public, max-age=3600")
            .body(bytes)
            .unwrap_or_else(|_| error_response(StatusCode::INTERNAL_SERVER_ERROR, "build response"));
        responder.respond(out);
    });
}

#[cfg(test)]
mod tests {
    use super::parse_cameo_uri;
    use tauri::http::Uri;

    #[test]
    fn parses_path_scoped_cameo_url() {
        let uri: Uri = "cameo://localhost/board-1/gen-20260526.png"
            .parse()
            .unwrap();
        let (board, rel) = parse_cameo_uri(&uri).unwrap();
        assert_eq!(board, "board-1");
        assert_eq!(rel, "gen-20260526.png");
    }

    #[test]
    fn parses_windows_webview2_custom_protocol_shape() {
        let uri: Uri = "http://cameo.localhost/board-1/gen-20260526.png"
            .parse()
            .unwrap();
        let (board, rel) = parse_cameo_uri(&uri).unwrap();
        assert_eq!(board, "board-1");
        assert_eq!(rel, "gen-20260526.png");
    }

    #[test]
    fn keeps_legacy_host_scoped_urls_working() {
        let uri: Uri = "cameo://board-1/gen-20260526.png".parse().unwrap();
        let (board, rel) = parse_cameo_uri(&uri).unwrap();
        assert_eq!(board, "board-1");
        assert_eq!(rel, "gen-20260526.png");
    }
}

#[cfg(test)]
mod ssrf_tests {
    use super::is_allowed_remote;

    #[test]
    fn allows_public_https() {
        assert!(is_allowed_remote("https://images.example.com/a.jpg"));
        assert!(is_allowed_remote("https://8.8.8.8/x"));
    }

    #[test]
    fn blocks_non_https_and_localhost() {
        assert!(!is_allowed_remote("http://images.example.com/a.jpg"));
        assert!(!is_allowed_remote("https://localhost/x"));
        assert!(!is_allowed_remote("https://foo.local/x"));
    }

    #[test]
    fn blocks_ipv4_private_and_loopback() {
        assert!(!is_allowed_remote("https://127.0.0.1/x"));
        assert!(!is_allowed_remote("https://10.0.0.1/x"));
        assert!(!is_allowed_remote("https://192.168.1.1/x"));
        assert!(!is_allowed_remote("https://169.254.169.254/latest/meta-data/")); // cloud metadata
    }

    #[test]
    fn blocks_ipv6_bracketed_bypass() {
        // The bug: host_str() returns "[::1]" etc.; without stripping brackets
        // these parsed as "not an IP" and were allowed.
        assert!(!is_allowed_remote("https://[::1]/x")); // loopback
        assert!(!is_allowed_remote("https://[::ffff:127.0.0.1]/x")); // v4-mapped loopback
        assert!(!is_allowed_remote("https://[fe80::1]/x")); // link-local
        assert!(!is_allowed_remote("https://[fc00::1]/x")); // unique-local
        assert!(!is_allowed_remote("https://[fd00::1]/x")); // unique-local
        assert!(!is_allowed_remote("https://[::]/x")); // unspecified
    }

    #[test]
    fn allows_public_ipv6() {
        assert!(is_allowed_remote("https://[2606:4700:4700::1111]/x")); // public DNS
    }
}

#[cfg(test)]
mod range_tests {
    use super::{parse_byte_range, MAX_RANGE_CHUNK};

    #[test]
    fn closed_range_is_inclusive() {
        assert_eq!(parse_byte_range("bytes=0-99", 1000), Some((0, 99, false)));
    }

    #[test]
    fn open_ended_runs_to_eof() {
        assert_eq!(parse_byte_range("bytes=500-", 1000), Some((500, 999, false)));
    }

    #[test]
    fn suffix_range_is_last_n_bytes_and_flagged() {
        assert_eq!(parse_byte_range("bytes=-200", 1000), Some((800, 999, true)));
    }

    #[test]
    fn end_is_clamped_to_size() {
        assert_eq!(parse_byte_range("bytes=0-99999", 1000), Some((0, 999, false)));
    }

    #[test]
    fn rejects_unsatisfiable_or_malformed() {
        assert_eq!(parse_byte_range("bytes=2000-3000", 1000), None);
        assert_eq!(parse_byte_range("bytes=abc", 1000), None);
        assert_eq!(parse_byte_range("bytes=0-9,20-29", 1000), None); // multi-range
        assert_eq!(parse_byte_range("bytes=0-0", 0), None); // empty resource
    }

    #[test]
    fn suffix_cap_keeps_the_tail_not_the_head() {
        // A `bytes=-N` request must return the LAST bytes even after the
        // MAX_RANGE_CHUNK cap — the regression was returning the head of the tail.
        let total = 100 * 1024 * 1024; // 100 MiB
        let (start, end, is_suffix) =
            parse_byte_range("bytes=-50000000", total).expect("valid suffix range");
        assert!(is_suffix);
        assert_eq!(end, total - 1); // ends at EOF
        // Apply the same cap the handler does for a suffix request:
        let capped_start = end.saturating_sub(MAX_RANGE_CHUNK - 1).max(start);
        assert_eq!(end, total - 1); // still ends at EOF
        assert_eq!(capped_start, total - MAX_RANGE_CHUNK); // last 16 MiB, not the first
    }
}
