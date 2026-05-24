/**
 * Anonymous usage telemetry.
 *
 * v1 sends one event per UTC day: `app_open`. The first send is what registers
 * the device on the server (see PRD v0.2 §0.2). Subsequent days re-touch the
 * device row's last_seen.
 *
 * Privacy invariants (PRD §1):
 *   - never sends file paths, prompts, image bytes, board names, or any PII;
 *   - opt-out toggle stops recurring sends but does NOT un-register the device;
 *   - all failures are silent (logged via front_log, not surfaced to the user).
 */

import { CLOUD_ENABLED, appVersion, cloudFetch, getDeviceId, platformTag } from "./index";
import { ipc } from "../../lib/ipc";
import type { AppConfig } from "../../types";

function todayUtc(): string {
  return new Date().toISOString().slice(0, 10);
}

/**
 * Boot-time entry: send `app_open` at most once per UTC day. Safe to call
 * many times — early-exits when cloud disabled / opted out / already sent.
 *
 * NB: even when telemetry is opted out, we register the device once on the
 * very first launch — gallery access depends on the device row existing.
 * The PRD calls this "identity issuance, not behavior tracking".
 */
export async function bootDailyPing(cfg: AppConfig): Promise<void> {
  if (!CLOUD_ENABLED) return;

  const today = todayUtc();

  // Already pinged today → nothing to do.
  if (cfg.last_telemetry_date === today) return;

  // Opt-out path: still do a one-time `device_register` ping so the server
  // upserts the device row, then never send `app_open` again.
  if (cfg.telemetry_opt_out) {
    if (cfg.last_telemetry_date) return; // already registered some time before
    await sendBatch([
      {
        event: "device_register",
        device_id: await getDeviceId(),
        platform: platformTag(),
        app_version: appVersion(),
        client_timestamp: new Date().toISOString(),
      },
    ]);
    await persistDate(cfg, today);
    return;
  }

  // Normal path: send today's app_open.
  await sendBatch([
    {
      event: "app_open",
      device_id: await getDeviceId(),
      platform: platformTag(),
      app_version: appVersion(),
      client_timestamp: new Date().toISOString(),
    },
  ]);
  await persistDate(cfg, today);
}

interface TrackEvent {
  event: string;
  device_id: string;
  platform: string;
  app_version: string;
  client_timestamp: string;
  props?: Record<string, unknown>;
}

async function sendBatch(events: TrackEvent[]): Promise<void> {
  try {
    await cloudFetch("/api/v1/events", {
      method: "POST",
      body: JSON.stringify({ events }),
      // POST events also doubles as the device-registration path on first
      // call; the server is the source of truth for whether a device exists,
      // so we let it (and the IP register limit) own that decision.
    });
    void log("info", `telemetry: sent ${events.length} event(s)`);
  } catch (e) {
    // Silent: don't bug the user about a backend hiccup.
    void log("warn", `telemetry: send failed (${(e as Error).message})`);
  }
}

async function persistDate(_cfg: AppConfig, date: string): Promise<void> {
  // Load-modify-store: re-read from disk so a concurrent setting edit (e.g.
  // user toggled the proxy while boot was in flight) is not clobbered by a
  // stale snapshot taken at boot. Telemetry only owns `last_telemetry_date`.
  try {
    const fresh = await ipc.cfgLoad();
    await ipc.cfgSave({ ...fresh, last_telemetry_date: date });
  } catch (e) {
    void log("warn", `telemetry: cfgSave failed (${(e as Error).message})`);
  }
}

async function log(level: "info" | "warn" | "error", msg: string): Promise<void> {
  try {
    await ipc.frontLog(level, msg);
  } catch {
    /* ignore */
  }
}
