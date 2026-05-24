/**
 * Cloud service entry — single source of truth for "is the cloud module
 * compiled in?" + the auth'd fetch wrapper used by every cameo.ink request.
 *
 * Two build-time env vars (Vite inlines them at compile time):
 *
 *   VITE_CAMEO_API_BASE   — e.g. https://cameo.ink
 *   VITE_CAMEO_API_KEY    — the key shared by all official builds of this
 *                            release; revocable server-side.
 *
 * Both unset (open-source `pnpm tauri build`) → `CLOUD_ENABLED === false`.
 * UI never renders Gallery / telemetry never sends. The whole module
 * short-circuits into a pure local tool.
 */

import { ipc } from "../../lib/ipc";

export const CLOUD_API_BASE = import.meta.env.VITE_CAMEO_API_BASE as
  | string
  | undefined;
export const CLOUD_API_KEY = import.meta.env.VITE_CAMEO_API_KEY as
  | string
  | undefined;
export const CLOUD_ENABLED = !!CLOUD_API_BASE && !!CLOUD_API_KEY;

// Memoised: hits Rust once per launch, reuses thereafter.
let cachedDeviceId: string | null = null;
export async function getDeviceId(): Promise<string> {
  if (cachedDeviceId) return cachedDeviceId;
  cachedDeviceId = await ipc.deviceIdGet();
  return cachedDeviceId;
}

/**
 * Drop the JS-side device-id memo so the next `getDeviceId` re-fetches from
 * Rust. Note: Rust caches its own copy in `OnceLock` for the lifetime of the
 * process, so even after `device_id_reset` removes the file, a same-session
 * `device_id_get` still returns the old uuid (the file is recreated by Rust
 * only after a fresh process start). The settings UI uses this clear to drop
 * its display cache; full identity rotation still requires an app restart.
 */
export function clearDeviceIdCache(): void {
  cachedDeviceId = null;
}

export function appVersion(): string {
  return __APP_VERSION__;
}

/** Coarse platform tag (no PII). UA sniff is enough for Tauri webview. */
export function platformTag(): string {
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes("mac")) return "macos";
  if (ua.includes("win")) return "windows";
  if (ua.includes("linux")) return "linux";
  return "unknown";
}

export class CloudError extends Error {
  constructor(public status: number, public code: string, msg?: string) {
    super(msg ?? code);
  }
}

/**
 * Fetch wrapper that injects auth headers and normalises errors.
 *
 *   401  → invalid / missing key (`CloudError.code === 'unauthorized'`)
 *   403  → device not registered / banned
 *   429  → daily quota exhausted
 *   5xx  → transient, throw with code='server_error' for callers to back off
 */
export async function cloudFetch(
  path: string,
  init?: RequestInit & { skipDeviceId?: boolean; signal?: AbortSignal },
): Promise<Response> {
  if (!CLOUD_ENABLED) throw new CloudError(0, "disabled", "cloud module not built in");
  const headers = new Headers(init?.headers);
  headers.set("X-API-Key", CLOUD_API_KEY!);
  if (!init?.skipDeviceId) {
    headers.set("X-Device-Id", await getDeviceId());
  }
  headers.set("X-App-Version", appVersion());
  if (init?.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  const url = `${CLOUD_API_BASE}${path}`;
  let res: Response;
  try {
    res = await fetch(url, { ...init, headers });
  } catch (e) {
    // Preserve AbortError so callers can distinguish "user navigated away"
    // from a real network failure (CloudError noise on close = bad UX).
    if ((e as Error).name === "AbortError") throw e;
    throw new CloudError(0, "network_error", (e as Error).message);
  }
  if (res.ok) return res;
  let bodyCode = "http_error";
  try {
    const body = (await res.clone().json()) as { error?: string };
    if (body?.error) bodyCode = body.error;
  } catch {
    /* ignore */
  }
  throw new CloudError(res.status, bodyCode);
}
