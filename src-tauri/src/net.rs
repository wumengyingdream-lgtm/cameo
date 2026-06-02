//! The single proxied HTTP client for ALL product network egress.
//!
//! WebView `fetch()` / `<img>` can't honor the app's Settings → Proxy (only the
//! OS proxy), and only an explicitly-configured `reqwest` client can. So every
//! outbound request that must respect the user's proxy — the cloud JSON API
//! (gallery, telemetry), proxied gallery images (cmnet:// scheme), and anything
//! added later — goes through [`client`].
//!
//! **Pit of success:** this is the only sanctioned way for Rust to make a
//! request, so a new network feature can't accidentally bypass the proxy. The
//! client is cached and rebuilt only when the proxy settings change.

use crate::proxy::ProxySettings;
use parking_lot::Mutex;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// SSRF-safe DNS resolver: resolves a hostname the normal way, then DROPS any
/// address in a private / loopback / link-local / unique-local range
/// (`protocol::is_blocked_ip`). reqwest connects only to the addresses this
/// returns, so a public hostname that *resolves* to 127.0.0.1 / a cloud
/// metadata IP / an intranet host (DNS-rebinding / private DNS) can't be
/// reached through `cmnet://` — closing the gap that the URL-string guard
/// (which only sees literal IPs) leaves open.
///
/// Applied to the shared `net::client` ONLY on the direct (no-proxy) path —
/// cloud API / gallery images / ffmpeg download, all legitimately public. With
/// a proxy enabled the proxy resolves the target, and reqwest would otherwise
/// use this resolver to reach the PROXY host itself (dropping a localhost/
/// private-DNS proxy and breaking everything), so it is NOT installed there;
/// SSRF protection then rests on the user's proxy. The auto-updater builds its
/// OWN client (updater.rs) and is unaffected either way.
struct SsrfGuardResolver;

impl reqwest::dns::Resolve for SsrfGuardResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            let resolved = tokio::net::lookup_host((host.as_str(), 0_u16)).await?;
            let allowed: Vec<SocketAddr> = resolved
                .filter(|a| !crate::protocol::is_blocked_ip(a.ip()))
                .collect();
            if allowed.is_empty() {
                let err: Box<dyn std::error::Error + Send + Sync> =
                    "host resolved only to blocked (private/loopback) addresses".into();
                return Err(err);
            }
            let iter: reqwest::dns::Addrs = Box::new(allowed.into_iter());
            Ok(iter)
        })
    }
}

static CLIENT: Mutex<Option<(ProxySettings, reqwest::Client)>> = Mutex::new(None);

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// A `reqwest::Client` with the current Settings → Proxy applied. Cheap to call
/// repeatedly: returns a cached clone, rebuilding only when the proxy config
/// changed since the last build.
pub fn client() -> reqwest::Client {
    let proxy = crate::config::load().proxy;
    let mut guard = CLIENT.lock();
    if let Some((cached, c)) = guard.as_ref() {
        if *cached == proxy {
            return c.clone();
        }
    }
    let c = build(&proxy);
    *guard = Some((proxy, c.clone()));
    c
}

fn build(proxy: &ProxySettings) -> reqwest::Client {
    let mut b = reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        // SSRF defense-in-depth: the `cmnet://` proxy only validates the INITIAL
        // URL (protocol.rs::is_allowed_remote). reqwest follows redirects by
        // default, so a public host could 302 to http://127.0.0.1 or the cloud
        // metadata IP. Re-validate every hop's destination with the same guard,
        // and forbid downgrade to non-https. Legit CDN/R2 redirects (public
        // https) pass untouched; this is shared by every caller, all of which
        // only ever talk to public https endpoints anyway. (URL-string based, so
        // it's safe with a proxy too — it doesn't resolve the proxy host.)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("too many redirects");
            }
            if crate::protocol::is_allowed_remote(attempt.url().as_str()) {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }));
    // A proxy is "installed" only if it's enabled AND its URL parsed AND reqwest
    // accepted it. Any other case (disabled, or an enabled-but-invalid config
    // that falls through) is a DIRECT connection and must get the direct-mode
    // SSRF guard — otherwise an invalid saved proxy would silently open the
    // `cmnet://` DNS-rebinding hole ([P2] fail closed on the fallback path).
    let mut proxy_installed = false;
    if proxy.enabled {
        match proxy.proxy_url() {
            Ok(url) => match reqwest::Proxy::all(&url) {
                Ok(p) => {
                    tracing::info!(module = "net", proxy = %url, "HTTP client using configured proxy");
                    b = b.proxy(p);
                    proxy_installed = true;
                }
                Err(e) => tracing::warn!(module = "net", "proxy url rejected ({e}); going direct"),
            },
            Err(e) => tracing::warn!(module = "net", "proxy enabled but invalid ({e}); going direct"),
        }
    }
    if !proxy_installed {
        // Direct path: ignore any inherited env proxy so the setting and the
        // actual transport agree (mirrors proxy.rs's env discipline), and install
        // the SSRF DNS guard. (We DON'T install it when a real proxy is set: the
        // proxy resolves the target, and reqwest would otherwise use this
        // resolver to reach the PROXY host itself — dropping a localhost/
        // private-DNS proxy and breaking all traffic. Under a proxy, SSRF
        // protection rests on the user's proxy.) The guard blocks hostnames that
        // resolve to private/loopback addresses (DNS rebinding), complementing
        // the literal-IP URL guard.
        b = b.no_proxy().dns_resolver(Arc::new(SsrfGuardResolver));
    }
    b.build().unwrap_or_else(|e| {
        tracing::warn!(module = "net", "client build failed ({e}); using default client");
        reqwest::Client::new()
    })
}
