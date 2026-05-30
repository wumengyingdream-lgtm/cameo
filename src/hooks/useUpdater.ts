/**
 * Cameo auto-updater bridge.
 *
 * Subscribes to the Rust updater's lifecycle events and exposes:
 *   - `pendingVersion`: non-null when there's a downloaded update ready to
 *     install. The titlebar button uses this to render itself.
 *   - `installing`: true while the install command is in flight (button shows
 *     a spinner, gets disabled to prevent double-click).
 *   - `restart()`: triggers the platform-specific install command. On macOS
 *     this just kills the Codex sidecar and `app.restart()`s. On Windows
 *     this hands the cached bytes to NSIS, which `exit(0)`s the process.
 *
 * No periodic check from JS — the Rust background loop owns that. We just
 * listen for `updater:ready-to-restart`. On Windows we also poll once at
 * mount for a pending-on-disk update from a previous session.
 */
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ipc } from "../lib/ipc";

export function useUpdater() {
  const [pendingVersion, setPendingVersion] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    // Live event: silent download finished (mac install done, or win bytes on disk).
    void listen<string>("updater:ready-to-restart", (event) => {
      if (cancelled) return;
      setPreparing(false);
      setError(null);
      setPendingVersion(event.payload || null);
    }).then((un) => unsubs.push(un));

    // A newer package is being downloaded/replacing the pending bytes. Hide
    // the install button until disk and the cached Update object align again.
    void listen<string>("updater:download-started", () => {
      if (cancelled) return;
      setPreparing(true);
      setError(null);
    }).then((un) => unsubs.push(un));

    // Live event: download failed. We don't surface anything to the user
    // (silent failures are an explicit product decision); if older pending
    // bytes still exist, the button can reappear and retry them.
    void listen<string>("updater:download-failed", () => {
      if (cancelled) return;
      setPreparing(false);
    }).then((un) => unsubs.push(un));

    void listen<string>("updater:install-failed", (event) => {
      if (cancelled) return;
      setInstalling(false);
      setPreparing(false);
      setError(event.payload || "install failed");
      void ipc.frontLog("warn", `update install failed: ${event.payload || "unknown"}`).catch(() => {});
    }).then((un) => unsubs.push(un));

    // Windows only: bytes on disk from a previous session — reveal the button
    // without re-checking the network.
    void invoke<string | null>("check_pending_update")
      .then((v) => {
        if (!cancelled && v) setPendingVersion(v);
      })
      .catch(() => {
        /* not on windows or no pending — silent */
      });

    return () => {
      cancelled = true;
      unsubs.forEach((u) => u());
    };
  }, []);

  const restart = useCallback(async () => {
    if (installing || !pendingVersion) return;
    setInstalling(true);
    try {
      await invoke("install_pending_update");
      // On success the process is replaced (mac relaunch / win NSIS exit);
      // we won't reach this code on the new process.
    } catch (e) {
      setInstalling(false);
      const msg = e instanceof Error ? e.message : String(e);
      setError(msg);
      void ipc.frontLog("warn", `install_pending_update failed: ${msg}`).catch(() => {});
    }
  }, [installing, pendingVersion]);

  return { pendingVersion: preparing ? null : pendingVersion, installing, error, restart };
}
