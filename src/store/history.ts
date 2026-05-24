import { create } from "zustand";

/** One reversible edit. Commands are recorded AFTER being applied, so `push`
 *  never re-runs `redo`; `undo`/`redo` move the command between stacks and run
 *  the matching thunk. The thunks call raw store mutations that persist but do
 *  NOT push history (no recursion). */
export interface Command {
  label: string;
  undo: () => void | Promise<void>;
  redo: () => void | Promise<void>;
}

const LIMIT = 200;

interface HistoryState {
  undoStack: Command[];
  redoStack: Command[];
  /** Record an already-applied command (clears the redo stack). */
  push: (cmd: Command) => void;
  undo: () => void;
  redo: () => void;
  clear: () => void;
}

export const useHistoryStore = create<HistoryState>((set, get) => ({
  undoStack: [],
  redoStack: [],

  push: (cmd) =>
    set((s) => ({ undoStack: [...s.undoStack, cmd].slice(-LIMIT), redoStack: [] })),

  undo: () => {
    const { undoStack } = get();
    const cmd = undoStack[undoStack.length - 1];
    if (!cmd) return;
    set((s) => ({ undoStack: s.undoStack.slice(0, -1), redoStack: [...s.redoStack, cmd] }));
    void cmd.undo();
  },

  redo: () => {
    const { redoStack } = get();
    const cmd = redoStack[redoStack.length - 1];
    if (!cmd) return;
    set((s) => ({ redoStack: s.redoStack.slice(0, -1), undoStack: [...s.undoStack, cmd] }));
    void cmd.redo();
  },

  clear: () => set({ undoStack: [], redoStack: [] }),
}));
