/**
 * Suspension-aware inactivity watchdog.
 *
 * Designed for the Codex turn lifecycle: starts when a turn begins, ticks at
 * `intervalMs`, and fires once after `timeoutMs` of silence. Every meaningful
 * runtime event calls `touch()` to reset the inactivity clock — so a healthy
 * turn (streaming text deltas, tool calls, image-gen placeholders) never trips
 * the watchdog. A wedged codex sidecar (network drop with no events) does.
 *
 * Two subtleties this class solves correctly:
 *
 *   1. **Suspension awareness**: macOS may suspend a backgrounded tab for
 *      minutes (App Nap, lid-close). Naïve `Date.now()` comparisons would
 *      treat the resumed wake-up as a 10-minute idle gap and falsely fire.
 *      We compare consecutive tick gaps; if `gap > 2 * intervalMs`, we know
 *      we were off-clock and re-baseline `lastActivity` to now without firing.
 *
 *   2. **Pausable for user think-time**: while the agent is awaiting a
 *      permission decision (or any other user-blocking interaction), the
 *      idle clock should NOT count. `pause()` / `resume()` are the levers;
 *      `resume()` re-baselines to now so a 5-minute confirmation prompt
 *      doesn't immediately trip the watchdog on resume.
 *
 * A browser-side timer — Cameo manages turn state in the JS chat store (not a
 * Node sidecar), so the inactivity clock lives here rather than server-side.
 */
export interface WatchdogOptions {
  /** Inactivity threshold before `onFire` runs. */
  timeoutMs: number;
  /** Tick frequency. Smaller = sharper detection, more wake-ups; larger = lazier
   *  but cheaper. 30 s is a good balance for a 10-min timeout. */
  intervalMs: number;
  /** Invoked when the inactivity threshold trips. Watchdog auto-`stop()`s
   *  before calling this — re-arm explicitly with `start()` if needed. */
  onFire: () => void;
}

export class InactivityWatchdog {
  private readonly opts: WatchdogOptions;
  private lastActivity = 0;
  private lastTick = 0;
  private intervalId: ReturnType<typeof setInterval> | null = null;
  private paused = false;

  constructor(opts: WatchdogOptions) {
    this.opts = opts;
  }

  /** Begin (or restart) watching. Re-baselines `lastActivity` to NOW. */
  start(): void {
    this.stop();
    const now = Date.now();
    this.lastActivity = now;
    this.lastTick = now;
    this.paused = false;
    this.intervalId = setInterval(() => this.tick(), this.opts.intervalMs);
  }

  /** Stop ticking. Idempotent. */
  stop(): void {
    if (this.intervalId !== null) {
      clearInterval(this.intervalId);
      this.intervalId = null;
    }
  }

  /** Whether the watchdog is currently armed. */
  get running(): boolean {
    return this.intervalId !== null;
  }

  /** Reset the inactivity clock. Call on every observed runtime event so a
   *  healthy turn (still emitting deltas, tool calls, etc.) never trips.
   *  No-op when the watchdog isn't running — a late event after `stop()`
   *  shouldn't silently re-arm the clock for a future `start()` call. */
  touch(): void {
    if (this.intervalId === null) return;
    this.lastActivity = Date.now();
  }

  /** Suspend evaluation. Use during user-blocking interactions (permission
   *  cards, confirmation modals). Activity events while paused are ignored. */
  pause(): void {
    this.paused = true;
  }

  /** Resume evaluation. Re-baselines `lastActivity` so the pause window
   *  doesn't count toward the timeout. */
  resume(): void {
    if (this.paused) {
      this.paused = false;
      const now = Date.now();
      this.lastActivity = now;
      this.lastTick = now;
    }
  }

  private tick(): void {
    const now = Date.now();
    const gap = now - this.lastTick;
    this.lastTick = now;

    if (this.paused) return;

    // Suspension detection: a tick gap far larger than the configured interval
    // means the OS suspended us (App Nap, lid close, sleep). Don't count the
    // off-clock time as inactivity — re-baseline and return.
    if (gap > this.opts.intervalMs * 2) {
      this.lastActivity = now;
      return;
    }

    if (now - this.lastActivity >= this.opts.timeoutMs) {
      // Auto-stop before firing so a slow onFire handler doesn't get
      // double-invoked on the next tick.
      this.stop();
      this.opts.onFire();
    }
  }
}
