import { create } from "zustand";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ipc } from "../lib/ipc";
import type { WorkspaceEntry } from "../types";
import { useBoardStore } from "./board";

interface WorkspaceState {
  workspaces: WorkspaceEntry[];
  sidebarOpen: boolean;
  refresh: () => Promise<void>;
  toggleSidebar: () => void;
  openWorkspace: (path: string) => Promise<void>;
  /** "+" — create a fresh board in the app area and open it. */
  newWorkspace: () => Promise<void>;
  /** "Add folder" — pick an external folder and open it. */
  addFolder: () => Promise<void>;
  rename: (id: string, name: string) => Promise<void>;
  remove: (id: string) => Promise<void>;
}

export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  workspaces: [],
  sidebarOpen: false,

  refresh: async () => {
    try {
      set({ workspaces: await ipc.listWorkspaces() });
    } catch {
      /* ignore */
    }
  },

  toggleSidebar: () => set((s) => ({ sidebarOpen: !s.sidebarOpen })),

  openWorkspace: async (path) => {
    await useBoardStore.getState().openBoard(path);
    await get().refresh();
  },

  newWorkspace: async () => {
    try {
      const path = await ipc.createWorkspace();
      await get().openWorkspace(path);
    } catch {
      /* ignore */
    }
  },

  addFolder: async () => {
    const path = await openDialog({ directory: true, multiple: false, title: "Add a folder as a workspace" });
    if (typeof path === "string") await get().openWorkspace(path);
  },

  rename: async (id, name) => {
    const trimmed = name.trim();
    if (!trimmed) return;
    try {
      await ipc.renameWorkspace(id, trimmed);
      if (useBoardStore.getState().boardId === id) useBoardStore.setState({ name: trimmed });
      await get().refresh();
    } catch {
      /* ignore */
    }
  },

  remove: async (id) => {
    try {
      await ipc.removeWorkspace(id);
      await get().refresh();
    } catch {
      /* ignore */
    }
  },
}));
