import { create } from "zustand";
import type { ClipItem } from "../types";

/**
 * Marker written to the OS clipboard (as text) on an in-app Cmd+C. Paste uses it
 * as a recency oracle: if the OS clipboard still carries this marker, our in-app
 * copy is the most recent one, so we paste the placements; if anything else has
 * since been copied (an image from another app, a file from Finder), the marker
 * is gone and we fall back to the OS clipboard. This keeps a stale in-app
 * clipboard from permanently shadowing external copies without eagerly clearing.
 *
 * No surrounding whitespace (some clipboard managers trim it) — paste compares
 * with `.trim()` for the same reason.
 */
export const CAMEO_CLIP_MARKER = "cameo-canvas-clipboard";

/**
 * In-app canvas clipboard for copy/paste of placements, including ACROSS boards
 * and for videos. The OS clipboard only holds a single rasterized image (and
 * cannot represent a video or a multi-selection), so a Cmd+C of N items would
 * otherwise paste just one flattened picture. This store carries the full
 * selection by source file path + transform; paste re-imports every item into
 * the target board (Rust copies the files, content-addressing dedups), so it
 * works for any media kind and any target board.
 */
interface ClipboardState {
  sourceBoardId: string | null;
  items: ClipItem[];
  /** Whether the OS-clipboard freshness marker was successfully written for the
   *  current copy. When false (writeText unavailable/denied), paste can't trust
   *  the marker, so it falls back to the in-app store rather than silently
   *  dropping a multi-select / video copy. */
  marked: boolean;
  set: (sourceBoardId: string, items: ClipItem[]) => void;
  setMarked: (marked: boolean) => void;
  clear: () => void;
}

export const useClipboardStore = create<ClipboardState>((set) => ({
  sourceBoardId: null,
  items: [],
  marked: false,
  set: (sourceBoardId, items) => set({ sourceBoardId, items, marked: false }),
  setMarked: (marked) => set({ marked }),
  clear: () => set({ sourceBoardId: null, items: [], marked: false }),
}));
