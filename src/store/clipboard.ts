import { create } from "zustand";
import type { ClipItem } from "../types";

/**
 * Marker written to the OS clipboard (as text) on an in-app Cmd+C. Paste uses it
 * as a recency oracle: if the OS clipboard still carries this marker, our in-app
 * copy is the most recent one, so we paste the placements; if anything else has
 * since been copied (an image from another app, a file from Finder), the marker
 * is gone and we fall back to the OS clipboard. This keeps a stale in-app
 * clipboard from permanently shadowing external copies without eagerly clearing.
 */
export const CAMEO_CLIP_MARKER = " cameo-canvas-clipboard ";

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
  set: (sourceBoardId: string, items: ClipItem[]) => void;
  clear: () => void;
}

export const useClipboardStore = create<ClipboardState>((set) => ({
  sourceBoardId: null,
  items: [],
  set: (sourceBoardId, items) => set({ sourceBoardId, items }),
  clear: () => set({ sourceBoardId: null, items: [] }),
}));
