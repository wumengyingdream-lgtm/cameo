/**
 * Tiny one-shot channels from outside-the-composer triggers (Gallery, chat
 * inline image right-click) into the composer's contentEditable.
 *
 * Two independent signals share the same store because the composer effect
 * watches a single nonce and dispatches on whichever payload is set.
 *
 * This is intentionally NOT a chat-store field: the composer is
 * contentEditable-native (no controlled state) and the chat store is about
 * session/turn state. Discrete signals keep the concerns separated.
 */

import { create } from "zustand";

/** A specific video frame to reference (PRD §17/F2): the source video's
 *  Placement is the pill, plus the extracted still + timestamp so the agent
 *  sees the exact moment. Absent → reference the whole file. */
export interface PendingFrameRef {
  /** Timestamp into the video, milliseconds (for ordering / display). */
  atMs: number;
  /** Board-relative path of the extracted still (from `reference_video_frame`). */
  path: string;
  /** Human label for the pill badge, e.g. "0:03". */
  label: string;
}

export interface PendingPill {
  placementId: string;
  frame?: PendingFrameRef;
}

interface ComposerState {
  /** Plain-text injection (Gallery "Use this prompt"). */
  pendingPrompt: string | null;
  /** Pill injection (chat inline image / video "reference") — the Placement to
   *  render as a reference pill at the caret, optionally a specific frame. */
  pendingPill: PendingPill | null;
  /** Bumped on every inject so the composer's `useEffect` re-fires even when
   *  the same prompt / pill is selected twice in a row (React skips effects
   *  whose dep values are referentially equal). */
  nonce: number;
  injectPrompt: (text: string) => void;
  injectPill: (placementId: string, frame?: PendingFrameRef) => void;
  consume: () => void;
}

export const useComposerStore = create<ComposerState>((set) => ({
  pendingPrompt: null,
  pendingPill: null,
  nonce: 0,
  injectPrompt: (text) =>
    set((s) => ({ pendingPrompt: text, pendingPill: null, nonce: s.nonce + 1 })),
  injectPill: (placementId, frame) =>
    set((s) => ({ pendingPill: { placementId, frame }, pendingPrompt: null, nonce: s.nonce + 1 })),
  consume: () => set({ pendingPrompt: null, pendingPill: null }),
}));
