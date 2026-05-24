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

interface ComposerState {
  /** Plain-text injection (Gallery "Use this prompt"). */
  pendingPrompt: string | null;
  /** Pill injection (chat inline image "Use as reference") — the existing
   *  Placement id to render as a reference pill at the caret. */
  pendingPill: string | null;
  /** Bumped on every inject so the composer's `useEffect` re-fires even when
   *  the same prompt / pill is selected twice in a row (React skips effects
   *  whose dep values are referentially equal). */
  nonce: number;
  injectPrompt: (text: string) => void;
  injectPill: (placementId: string) => void;
  consume: () => void;
}

export const useComposerStore = create<ComposerState>((set) => ({
  pendingPrompt: null,
  pendingPill: null,
  nonce: 0,
  injectPrompt: (text) =>
    set((s) => ({ pendingPrompt: text, pendingPill: null, nonce: s.nonce + 1 })),
  injectPill: (placementId) =>
    set((s) => ({ pendingPill: placementId, pendingPrompt: null, nonce: s.nonce + 1 })),
  consume: () => set({ pendingPrompt: null, pendingPill: null }),
}));
