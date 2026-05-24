//! Global app config — `~/.cameo/config.json`. App-level settings that are NOT
//! tied to any one Board (currently: the network proxy). Atomic tmp+rename
//! write, like the Board doc (storage.rs).

use crate::paths::app_config_path;
use crate::proxy::ProxySettings;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub proxy: ProxySettings,
    /// Disable anonymous usage telemetry (default: false = enabled).
    /// Note: the one-time device registration on first launch is identity
    /// issuance, not behavior tracking, and is not gated by this flag.
    pub telemetry_opt_out: bool,
    /// Last date we sent an `app_open` event, ISO-8601 YYYY-MM-DD. Used to
    /// dedupe daily so a single device pings the server at most once per day.
    pub last_telemetry_date: Option<String>,
    /// Closing the window hides it to the tray instead of quitting (default:
    /// true). Read at window-close time by the close handler in `lib.rs`.
    pub close_to_tray: bool,
}

// Manual `Default` (not derived) so `close_to_tray` defaults to `true`. With
// `#[serde(default)]` on the container, configs missing the field also inherit
// this — so existing installs and fresh launches both start with tray-on.
impl Default for AppConfig {
    fn default() -> Self {
        Self {
            proxy: ProxySettings::default(),
            telemetry_opt_out: false,
            last_telemetry_date: None,
            close_to_tray: true,
        }
    }
}

/// Load `config.json`; a missing or corrupt file yields defaults (never fails).
pub fn load() -> AppConfig {
    match std::fs::read(app_config_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!(module = "config", "config.json parse failed ({e}); using defaults");
            AppConfig::default()
        }),
        Err(_) => AppConfig::default(),
    }
}

pub fn save(cfg: &AppConfig) -> Result<()> {
    let p = app_config_path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(cfg)?;
    std::fs::write(&tmp, &json).with_context(|| format!("write {}", tmp.display()))?;
    std::fs::rename(&tmp, &p).with_context(|| format!("rename to {}", p.display()))?;
    Ok(())
}
