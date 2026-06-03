import { create } from "zustand";

/**
 * Live state of the on-canvas video player. Only one video is "live" at a time
 * (the focused VideoOverlay registers itself here). This is the bridge that
 * lets the imperative PixiJS scene (spacebar → play/pause) and the SelectionBar
 * ("reference" button → whole-file vs. current frame) read/drive the focused
 * video without reaching into React internals.
 */
interface VideoPlaybackState {
  /** Placement id of the currently-focused video, or null when none is live. */
  placementId: string | null;
  currentTime: number;
  duration: number;
  playing: boolean;
  /** True once the user has scrubbed this video — "reference" then sends that
   *  specific frame instead of the whole file. Cleared on play / video change. */
  scrubbed: boolean;
  /** Toggle play/pause on the live video (provided by VideoOverlay). */
  toggle: (() => void) | null;
  register: (placementId: string, toggle: () => void) => void;
  unregister: (placementId: string) => void;
  setTime: (t: number, scrubbed?: boolean) => void;
  setDuration: (d: number) => void;
  setPlaying: (p: boolean) => void;
}

const idle = {
  placementId: null,
  toggle: null,
  currentTime: 0,
  duration: 0,
  playing: false,
  scrubbed: false,
} as const;

export const useVideoPlaybackStore = create<VideoPlaybackState>((set) => ({
  ...idle,
  register: (placementId, toggle) => set({ ...idle, placementId, toggle }),
  unregister: (placementId) =>
    set((s) => (s.placementId === placementId ? { ...idle } : {})),
  setTime: (t, scrubbed) => set((s) => ({ currentTime: t, scrubbed: scrubbed ? true : s.scrubbed })),
  setDuration: (d) => set({ duration: d }),
  // Playing implies "watch the whole clip" → a fresh reference is the whole file.
  setPlaying: (p) => set(p ? { playing: true, scrubbed: false } : { playing: false }),
}));
