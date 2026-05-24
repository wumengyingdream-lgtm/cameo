//! Unified logging. Every tech-stack layer funnels into ONE place so a
//! developer/AI can grep a single spot to debug:
//!
//!   - Rust backend (`tracing::*` with a `module = "…"` field)
//!   - Webview/React (bridged via the `front_log` command → `module = "front"`)
//!   - Codex sidecar stderr (drained → `module = "codex-stderr"`, see codex.rs)
//!
//! Two sinks share one filter:
//!   1. stderr — for `pnpm tauri dev` console.
//!   2. a daily-rolling file at `~/.cameo/logs/cameo.log.YYYY-MM-DD` (kept 14
//!      days), written through a non-blocking worker.
//!
//! Default verbosity: `info` everywhere + `debug` for our own crate. Override
//! with `RUST_LOG` (e.g. `RUST_LOG=cameo_lib=trace`).
//!
//! `init()` returns the appender's `WorkerGuard`; the caller MUST keep it alive
//! for the process lifetime (drop = the file sink stops flushing).

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn filter() -> EnvFilter {
    EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,cameo_lib=debug"))
}

#[must_use = "keep the WorkerGuard alive for the whole app, or file logs stop flushing"]
pub fn init() -> Option<WorkerGuard> {
    let stderr_layer = fmt::layer().with_writer(std::io::stderr).with_target(false);

    let file = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("cameo")
        .filename_suffix("log")
        .max_log_files(14)
        .build(crate::paths::cameo_logs_dir());

    match file {
        Ok(appender) => {
            let (nb, guard) = tracing_appender::non_blocking(appender);
            let file_layer = fmt::layer().with_writer(nb).with_ansi(false).with_target(false);
            let _ = tracing_subscriber::registry()
                .with(filter())
                .with(stderr_layer)
                .with(file_layer)
                .try_init();
            Some(guard)
        }
        Err(e) => {
            // File sink unavailable — degrade to stderr-only rather than crash.
            let _ = tracing_subscriber::registry().with(filter()).with(stderr_layer).try_init();
            eprintln!("[logging] rolling file init failed: {e}; logging to stderr only");
            None
        }
    }
}
