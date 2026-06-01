/**
 * Per-turn telemetry accumulator for the Tier-1 `ai_turn_complete` event.
 *
 * A "turn" (= one AI query) spans several runtime events: it starts on
 * `chat.startTurn` and ends on `turnComplete`/`error`, with `usage` (tokens) and
 * `imageGenerated` arriving in between. This module collects that per-turn state
 * and emits exactly one `ai_turn_complete` at the end.
 *
 * Driven entirely from `src/store/chat.ts` (the single turn funnel) — see
 * specs/tech_docs/analytics_events.md §2 in cameo_web. No-op in open-source
 * builds; `track()` itself also honors the telemetry opt-out.
 */

import { CLOUD_ENABLED } from "./index";
import { track } from "./telemetry";

interface TurnMeta {
  model: string;
  effort: string;
  serviceTier: string | null;
  hasRefs: boolean;
}

interface TurnAcc extends TurnMeta {
  t0: number;
  images: number;
  inputTokens: number;
  outputTokens: number;
}

// At most one turn is in flight per process. Terminal paths settle it explicitly:
// turnComplete/error via the chat.ts taps, and abnormal teardowns (watchdog
// restart, Stop-no-response, sessionComplete-mid-turn, local transport failure)
// via endTurn() in forceRestartSession/failLocalTurn. A fresh beginTurn still
// supersedes any accumulator abandoned without ANY terminal event (e.g. board
// switch mid-turn), so we never leak metrics across turns.
let acc: TurnAcc | null = null;

/** Start measuring a turn. Reads model/effort/tier from the caller (chat.ts). */
export function beginTurn(meta: TurnMeta): void {
  if (!CLOUD_ENABLED) return;
  acc = { ...meta, t0: Date.now(), images: 0, inputTokens: 0, outputTokens: 0 };
}

/** One image produced this turn. */
export function noteImage(): void {
  if (acc) acc.images += 1;
}

/** `usage` reports the turn's token totals; latest wins (set, not add). */
export function noteUsage(inputTokens: number, outputTokens: number): void {
  if (!acc) return;
  if (Number.isFinite(inputTokens)) acc.inputTokens = inputTokens;
  if (Number.isFinite(outputTokens)) acc.outputTokens = outputTokens;
}

/** Terminal: emit `ai_turn_complete` and clear. Idempotent (second call no-ops). */
export function endTurn(status: string, error?: string | null): void {
  const a = acc;
  acc = null;
  if (!a) return;
  const op = a.hasRefs ? "edit" : a.images > 0 ? "generate" : "chat";
  const props: Record<string, unknown> = {
    status,
    model: a.model,
    effort: a.effort,
    service_tier: a.serviceTier ?? "standard",
    input_tokens: a.inputTokens,
    output_tokens: a.outputTokens,
    duration_ms: Date.now() - a.t0,
    image_count: a.images,
    op,
    has_refs: a.hasRefs,
  };
  // Coarse, non-PII error bucket only — never the raw message.
  if (status !== "completed") props.error_kind = classifyError(error);
  void track("ai_turn_complete", props);
}

function classifyError(msg?: string | null): string {
  const m = (msg ?? "").toLowerCase();
  if (/rate|limit|429|quota/.test(m)) return "rate_limit";
  if (/network|timeout|connect|offline|dns|socket/.test(m)) return "network";
  if (/auth|login|credential|unauthor|401|403/.test(m)) return "auth";
  return "other";
}
