/**
 * Playful "agent is working" copy pool for the ambient streaming-status
 * indicator. Themed around the actual product domain (image generation /
 * canvas / Codex agent) plus light self-deprecation. Deliberately not stuffed
 * with generic programming memes — those don't match Cameo's voice.
 *
 * Selection is deterministic given a `seed` (an integer that increments on
 * meaningful state transitions and on a timer): same seed → same phrase,
 * which is what we want during animation frames. Bumping the seed picks the
 * next phrase from the pool in a stable cycle, so a user who happens to
 * watch closely won't see the same line twice in a row.
 */

const ZH_GENERAL = [
  "灵感酝酿中…",
  "苦思冥想中…",
  "小脑袋瓜转啊转…",
  "容我先想想…",
  "调色板擦干净中…",
  "笔尖正在沾墨…",
  "模特还在化妆…",
  "翻参考图册中…",
  "给画布打底色…",
  "像素们排队入座…",
  "让阴影更阴一点…",
  "噪声正在退散…",
  "灵感咖啡冲泡中…",
  "画笔正在试色…",
  "正在取景…",
  "调整构图中…",
  "给图层排排队…",
  "画到细节就慢一些…",
  "装作很努力…",
  "我可不是在划水…",
  "画工不输蒙娜丽莎…",
  "别催，好画慢慢磨…",
  "让灵感再多酝酿一会儿…",
  "笔尖正在抖动…",
];

const EN_GENERAL = [
  "Brewing inspiration…",
  "Thinking really hard…",
  "Let me think about it…",
  "Pondering…",
  "Mixing pigments…",
  "Dipping the brush…",
  "Sketching first…",
  "Studying references…",
  "Priming the canvas…",
  "Aligning pixels…",
  "Adjusting shadows…",
  "Denoising vibes…",
  "Brewing genius coffee…",
  "Testing color swatches…",
  "Framing the shot…",
  "Reworking the composition…",
  "Layering up…",
  "Slowing down for the details…",
  "Pretending to work hard…",
  "Definitely not slacking…",
  "Mona Lisa was easier…",
  "Good art takes time…",
  "Letting the muse marinate…",
  "Steady the hand…",
];

/** A narrower image-themed subset used when the user's active block is
 *  specifically image generation. Stays on-theme rather than randomly
 *  drifting to "loading mystery algorithm…" mid-stroke. */
const ZH_IMAGE = [
  "调色板擦干净中…",
  "笔尖正在沾墨…",
  "给画布打底色…",
  "让阴影更阴一点…",
  "噪声正在退散…",
  "画笔正在试色…",
  "调整构图中…",
  "给图层排排队…",
  "画到细节就慢一些…",
  "笔尖正在抖动…",
];

const EN_IMAGE = [
  "Mixing pigments…",
  "Dipping the brush…",
  "Priming the canvas…",
  "Adjusting shadows…",
  "Denoising vibes…",
  "Testing color swatches…",
  "Reworking the composition…",
  "Layering up…",
  "Slowing down for the details…",
  "Steady the hand…",
];

export type PhraseLocale = "zh" | "en";
export type PhrasePool = "general" | "image";

/** Pick a phrase deterministically from the locale × pool grid. */
export function pickPhrase(locale: PhraseLocale, pool: PhrasePool, seed: number): string {
  const list =
    locale === "zh"
      ? pool === "image"
        ? ZH_IMAGE
        : ZH_GENERAL
      : pool === "image"
        ? EN_IMAGE
        : EN_GENERAL;
  // Stable, non-negative index. Modulo on negative seed produces negative in JS;
  // (((n % len) + len) % len) is the safe normalised form.
  const idx = ((seed % list.length) + list.length) % list.length;
  return list[idx];
}

/** Specific overrides that take priority over the random pool. Used for
 *  semantically loaded states where a fun random phrase would be misleading. */
export function fixedPhrase(
  locale: PhraseLocale,
  state: "starting" | "thinking" | "tool",
  detail?: string,
): string {
  if (locale === "zh") {
    if (state === "starting") return "Codex 启动中…（首次较慢）";
    if (state === "thinking") return "正在思考…";
    if (state === "tool") return detail ? `调用 ${detail} 中…` : "调用工具中…";
  } else {
    if (state === "starting") return "Codex starting up… (first launch is slower)";
    if (state === "thinking") return "Thinking…";
    if (state === "tool") return detail ? `Calling ${detail}…` : "Running a tool…";
  }
  return "";
}
