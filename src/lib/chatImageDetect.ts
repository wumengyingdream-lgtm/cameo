/**
 * Detect image references inside an assistant message's streamed text.
 *
 * Two recognition modes (decision Q1 = "B"):
 *
 *   • **Markdown**: `![alt](path)` (image syntax) AND `[text](path)` (plain
 *     link syntax) — both accepted when `path` ends in a known image
 *     extension. Models often emit the link form instead of the image form,
 *     and we still want the path consumed (otherwise the brackets and parens
 *     leak as raw text around the inline thumbnail). Closing `)` required so
 *     mid-stream partials like `[foo](./bar.` never half-render (Q5).
 *
 *   • **Plain path**: any whitespace-bounded token containing at least one `/`
 *     and ending in a known image extension. The `/` requirement keeps us
 *     out of false positives where the model casually mentions filenames
 *     ("you can save this to output.png") — those bare filenames are not
 *     recognised. Markdown is more permissive (any path shape inside `()`).
 *
 * Returns ordered, non-overlapping ranges into the source string so callers
 * can slice the original text and render each segment as either plain text
 * or an inline image card.
 */

export interface ImageRef {
  /** Start index in the source text (inclusive). */
  start: number;
  /** End index (exclusive). */
  end: number;
  kind: "markdown" | "plain";
  /** The raw path string as written by the model. May be relative, absolute,
   *  or tilde-prefixed — resolution happens server-side via `resolve_chat_image`. */
  path: string;
  /** Markdown alt text (kind === "markdown" only). */
  alt?: string;
}

const IMAGE_EXTS = "(?:png|jpe?g|webp|gif|bmp|tiff?|avif)";

// Markdown image / link to image: `![alt](image.png)` or `[text](image.png)`.
// `!?` allows both forms; the path *must* end in a known image extension so
// non-image links (`[docs](readme.md)`) don't get pulled in. Closing `)` is
// required (Q5: never half-render). `[^)]+` stops at the first `)` so the
// match doesn't run away across the rest of the message.
const MD_RE = new RegExp(
  String.raw`!?\[([^\]]*)\]\(([^)]+\.${IMAGE_EXTS})\)`,
  "gi",
);

// Plain path token: must contain `/`, end in image ext. Whitespace-bounded so
// punctuation inside path components doesn't bleed in. The leading lookbehind
// (^|\s|[`"']) and trailing lookahead allow common surroundings (backticks,
// quotes, commas) without consuming them as part of the path.
const PLAIN_RE = new RegExp(
  String.raw`(?<=^|[\s\`"'(])([^\s\`"'()<>]*\/[^\s\`"'()<>]*\.${IMAGE_EXTS})(?=[\s\`"'),.!?:;]|$)`,
  "gi",
);

export function extractImageRefs(text: string): ImageRef[] {
  const refs: ImageRef[] = [];

  // Markdown first — they take precedence over plain (so the path inside `()`
  // is never re-matched as a plain ref).
  for (const m of text.matchAll(MD_RE)) {
    const idx = m.index ?? 0;
    refs.push({
      start: idx,
      end: idx + m[0].length,
      kind: "markdown",
      path: m[2].trim(),
      alt: m[1],
    });
  }

  // Plain — skip anything overlapping a markdown match.
  for (const m of text.matchAll(PLAIN_RE)) {
    const idx = m.index ?? 0;
    const start = idx;
    const end = start + m[1].length;
    if (refs.some((r) => r.kind === "markdown" && start < r.end && end > r.start)) continue;
    refs.push({ start, end, kind: "plain", path: m[1] });
  }

  refs.sort((a, b) => a.start - b.start);
  return refs;
}
