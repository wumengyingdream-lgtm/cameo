//! HTTP/SOCKS5 proxy injection for the Codex sidecar (ported from a sibling
//! Tauri app's `proxy.rs`).
//!
//! GUI-launched apps inherit the Finder/launchd env, not the user's terminal
//! env — so a proxy exported in the shell never reaches the `codex app-server`
//! child. This module lets the user configure one proxy in Settings and inject
//! it reliably into every Codex spawn. Key invariants:
//!
//! 1. Inject **both casings** of HTTP_PROXY / HTTPS_PROXY / NO_PROXY — different
//!    HTTP stacks (reqwest, curl, openssl) read different ones.
//! 2. When ENABLED, set `ALL_PROXY` to the SAME url as HTTP(S)_PROXY. Codex
//!    streams model responses over a WebSocket (`wss://`), and WS transports in
//!    many stacks ignore HTTP(S)_PROXY and only honor ALL_PROXY — so without it
//!    the HTTPS handshake is proxied (works) but the stream goes direct and gets
//!    dropped ("websocket closed by server before response.completed"). Setting
//!    ALL_PROXY to the same value can't shadow HTTP(S)_PROXY (they match), so
//!    we get full coverage without the stale-value footgun. When DISABLED we
//!    still `env_remove` ALL_PROXY so an inherited stale value can't reroute.
//! 3. Always inject `NO_PROXY=localhost,...` — even when disabled — so an
//!    inherited system proxy never reroutes localhost.
//! 4. Fail-safe on invalid config: strip all proxy env rather than leaving a
//!    partial / dangerous state.

use serde::{Deserialize, Serialize};

/// Localhost coverage: DNS names + IPv4 loopback range + IPv6 loopback.
const LOCALHOST_NO_PROXY: &str = "localhost,localhost.localdomain,127.0.0.1,127.0.0.0/8,::1,[::1]";

const ALLOWED_PROTOCOLS: &[&str] = &["http", "https", "socks5"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySettings {
    pub enabled: bool,
    /// "http" | "https" | "socks5". UI exposes http / socks5; https accepted for
    /// hand-edits.
    pub protocol: String,
    pub host: String,
    pub port: u16,
}

impl Default for ProxySettings {
    fn default() -> Self {
        // Mirrors the common local proxy default (Clash/Mihomo mixed port).
        Self { enabled: false, protocol: "http".into(), host: "127.0.0.1".into(), port: 7897 }
    }
}

impl ProxySettings {
    /// Compose `<protocol>://<host>:<port>` if valid; Err on any validation
    /// failure (empty host, port 0, unknown protocol, URL-shaped host).
    pub fn proxy_url(&self) -> Result<String, &'static str> {
        let proto = self.protocol.trim().to_lowercase();
        if !ALLOWED_PROTOCOLS.contains(&proto.as_str()) {
            return Err("invalid protocol (use http / https / socks5)");
        }
        let host = self.host.trim();
        if host.is_empty() {
            return Err("empty host");
        }
        if host.contains("://") || host.contains('@') || host.contains('/') {
            return Err("host should not contain scheme / path");
        }
        if host.len() > 253 {
            return Err("host too long");
        }
        if self.port == 0 {
            return Err("port must be 1-65535");
        }
        Ok(format!("{proto}://{host}:{port}", port = self.port))
    }
}

/// Mutate `cmd` so the Codex child inherits the configured proxy (or no proxy +
/// localhost protection when none is configured).
pub fn apply_to_subprocess(cmd: &mut tokio::process::Command, cfg: Option<&ProxySettings>) {
    match cfg {
        Some(cfg) if cfg.enabled => match cfg.proxy_url() {
            Ok(url) => {
                cmd.env("HTTP_PROXY", &url);
                cmd.env("HTTPS_PROXY", &url);
                cmd.env("http_proxy", &url);
                cmd.env("https_proxy", &url);
                // ALL_PROXY (same url) so the wss model stream is proxied too —
                // see invariant 2. Safe to set since it matches HTTP(S)_PROXY.
                cmd.env("ALL_PROXY", &url);
                cmd.env("all_proxy", &url);
                cmd.env("NO_PROXY", LOCALHOST_NO_PROXY);
                cmd.env("no_proxy", LOCALHOST_NO_PROXY);
                tracing::info!(module = "proxy", proxy_url = %url, "proxy injected into codex sidecar env (incl. ALL_PROXY for wss stream)");
            }
            Err(e) => {
                tracing::warn!(module = "proxy", "proxy enabled but invalid ({e}); stripping all proxy env");
                strip_all_proxy_env(cmd);
            }
        },
        _ => {
            cmd.env("NO_PROXY", LOCALHOST_NO_PROXY);
            cmd.env("no_proxy", LOCALHOST_NO_PROXY);
            cmd.env_remove("ALL_PROXY");
            cmd.env_remove("all_proxy");
        }
    }
}

fn strip_all_proxy_env(cmd: &mut tokio::process::Command) {
    for var in [
        "HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY", "all_proxy",
        "NO_PROXY", "no_proxy",
    ] {
        cmd.env_remove(var);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(enabled: bool, proto: &str, host: &str, port: u16) -> ProxySettings {
        ProxySettings { enabled, protocol: proto.into(), host: host.into(), port }
    }

    #[test]
    fn url_compose_http() {
        assert_eq!(make(true, "http", "127.0.0.1", 7890).proxy_url().unwrap(), "http://127.0.0.1:7890");
    }

    #[test]
    fn url_compose_socks5() {
        assert_eq!(
            make(true, "socks5", "proxy.example.com", 1080).proxy_url().unwrap(),
            "socks5://proxy.example.com:1080"
        );
    }

    #[test]
    fn url_normalizes_protocol_case() {
        assert_eq!(make(true, "HTTP", "127.0.0.1", 7890).proxy_url().unwrap(), "http://127.0.0.1:7890");
    }

    #[test]
    fn url_trims_host_whitespace() {
        assert_eq!(make(true, "http", "  127.0.0.1  ", 7890).proxy_url().unwrap(), "http://127.0.0.1:7890");
    }

    #[test]
    fn url_rejects_empty_host() {
        assert!(make(true, "http", "", 7890).proxy_url().is_err());
    }

    #[test]
    fn url_rejects_unknown_protocol() {
        assert!(make(true, "ftp", "127.0.0.1", 21).proxy_url().is_err());
    }

    #[test]
    fn url_rejects_zero_port() {
        assert!(make(true, "http", "127.0.0.1", 0).proxy_url().is_err());
    }

    #[test]
    fn url_rejects_url_shaped_host() {
        assert!(make(true, "http", "http://127.0.0.1", 8080).proxy_url().is_err());
        assert!(make(true, "http", "user@proxy", 8080).proxy_url().is_err());
        assert!(make(true, "http", "host/path", 8080).proxy_url().is_err());
    }
}
