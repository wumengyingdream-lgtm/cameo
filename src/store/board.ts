import { create } from "zustand";
import { ipc } from "../lib/ipc";
import { useHistoryStore } from "./history";
import { useUiStore } from "./ui";
import { useToastStore } from "./toast";
import { getMessage } from "../i18n/locale";
import type { Asset, ImportResult, Placement, PlacementUpdate, Shape } from "../types";

/** A video minted without a poster means ffmpeg was unavailable at import time.
 *  Kick off a one-time managed install (detect-first; downloads only if truly
 *  missing) so playback/scrub/poster light up — App listens for `ffmpeg:done`
 *  to backfill posters. Safe to call repeatedly: the Rust install guards against
 *  concurrent runs and no-ops when ffmpeg is already present. */
function maybeInstallFfmpegFor(assets: Asset[]): void {
  if (!assets.some((a) => a.mime.startsWith("video/") && !a.posterPath)) return;
  void ipc.toolStatus().then((st) => {
    if (st.state === "ready" || st.state === "installing") return;
    useToastStore.getState().show(getMessage("ffmpeg.installing"), "info");
    // toolInstall resolves after the install completes; backfill posters for the
    // videos that minted poster-less so they stop showing as placeholder tiles.
    void ipc
      .toolInstall()
      .then(() => useBoardStore.getState().backfillVideoPosters())
      .catch(() => undefined);
  });
}

export interface GenPlaceholder {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface BoardState {
  boardId: string | null;
  folder: string | null;
  name: string | null;
  assets: Map<string, Asset>;
  placements: Map<string, Placement>;
  /** placementId → annotation shapes. */
  annotations: Map<string, Shape[]>;
  /** Transient "generating…" loading placeholders (not persisted). */
  placeholders: Map<string, GenPlaceholder>;
  selection: Set<string>;
  opening: boolean;
  error: string | null;

  openBoard: (path: string) => Promise<void>;
  /** Import + place; returns the new placements (so the composer can pill them). */
  importFiles: (paths: string[]) => Promise<Placement[]>;
  importFilesAt: (paths: string[], center: { x: number; y: number }) => Promise<Placement[]>;
  importBytes: (bytes: Uint8Array, ext: string, stem: string) => Promise<Placement[]>;
  importBytesAt: (bytes: Uint8Array, ext: string, stem: string, center: { x: number; y: number }) => Promise<Placement[]>;
  /** Merge an already-completed Rust import (assets + placements) into local
   *  state, pushing the standard import-undo step. Used by side paths that
   *  invoked their own dedicated Rust importer (e.g. `import_chat_image_to_canvas`)
   *  but still want the board to reflect the result + be undoable. */
  applyImportResult: (result: ImportResult) => void;
  /** Commit final transforms after a drag/scale gesture (persists). */
  commitMoves: (updates: PlacementUpdate[]) => Promise<void>;
  /** Mirror of the canvas selection (authoritative selection lives in the scene). */
  setSelection: (ids: string[]) => void;
  deleteSelected: () => Promise<void>;
  /** Add a Codex-generated Asset+Placement and select it (so the next turn
   *  re-grounds on the latest output — gap A3). Removes the loading placeholder
   *  it replaces. Rust already persisted it. */
  addGenerated: (asset: Asset, placement: Placement, placeholderId?: string | null) => void;
  /** Replace a Placement's annotation shapes (persists via IPC). */
  setAnnotation: (placementId: string, shapes: Shape[]) => void;
  /** Clear marks on the given images after they've been sent (no undo entry). */
  consumeMarks: (ids: string[]) => void;
  /** Replace a placement's image with new PNG bytes (crop bake); undoable. */
  replacePlacementImage: (id: string, bytes: number[]) => Promise<void>;
  /** Show a transient "generating…" loading placeholder on the canvas. */
  addPlaceholder: (id: string, rect: GenPlaceholder) => void;
  /** Drop all loading placeholders (e.g. on turn end). */
  clearPlaceholders: () => void;
  /** Select a placement and ask the canvas to center/focus on it. */
  revealRequest: { id: string; n: number } | null;
  revealPlacement: (id: string) => void;
  /** Rename the file backing a placement's asset (persists; updates path). */
  renameAsset: (placementId: string, newName: string) => Promise<void>;
  /** Extract a still from a video placement (PNG bytes captured from <video>)
   *  → new image Asset placed right-of-video with lineage; selects it. */
  extractFrame: (placementId: string, bytes: Uint8Array) => Promise<void>;
  /** Merge poster/metadata updates from a post-install ffmpeg backfill so the
   *  canvas swaps placeholder tiles for real stills (no reopen). */
  backfillVideoPosters: () => Promise<void>;
}

function indexById<T extends { id: string }>(items: T[]): Map<string, T> {
  return new Map(items.map((it) => [it.id, it]));
}

/** Record an undo entry for an import (remove the placements / restore them). */
function pushImportUndo(
  boardId: string,
  imported: Placement[],
  set: (updater: (s: BoardState) => Partial<BoardState>) => void
) {
  if (!imported.length) return;
  const ids = imported.map((p) => p.id);
  const remove = () => {
    set((s) => {
      const next = new Map(s.placements);
      for (const id of ids) next.delete(id);
      return { placements: next };
    });
    void ipc.deletePlacements(boardId, ids).catch(() => undefined);
  };
  const add = () => {
    set((s) => {
      const next = new Map(s.placements);
      for (const p of imported) next.set(p.id, p);
      return { placements: next };
    });
    void ipc.restorePlacements(boardId, imported).catch(() => undefined);
  };
  useHistoryStore.getState().push({ label: "Import", undo: remove, redo: add });
}

function mergeImport(state: BoardState, result: ImportResult) {
  const assets = new Map(state.assets);
  for (const a of result.assets) assets.set(a.id, a);
  const placements = new Map(state.placements);
  for (const p of result.placements) placements.set(p.id, p);
  return { assets, placements };
}

function centerImport(state: BoardState, result: ImportResult, center: { x: number; y: number }): ImportResult {
  if (result.placements.length === 0) return result;
  const assets = new Map(state.assets);
  for (const a of result.assets) assets.set(a.id, a);
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const p of result.placements) {
    const asset = assets.get(p.assetId);
    const halfW = ((asset?.width ?? 1) * p.scale) / 2;
    const halfH = ((asset?.height ?? 1) * p.scale) / 2;
    minX = Math.min(minX, p.x - halfW);
    minY = Math.min(minY, p.y - halfH);
    maxX = Math.max(maxX, p.x + halfW);
    maxY = Math.max(maxY, p.y + halfH);
  }
  if (!Number.isFinite(minX) || !Number.isFinite(minY) || !Number.isFinite(maxX) || !Number.isFinite(maxY)) {
    return result;
  }
  const dx = center.x - (minX + maxX) / 2;
  const dy = center.y - (minY + maxY) / 2;
  return {
    assets: result.assets,
    placements: result.placements.map((p) => ({ ...p, x: p.x + dx, y: p.y + dy })),
  };
}

export const useBoardStore = create<BoardState>((set, get) => ({
  boardId: null,
  folder: null,
  name: null,
  assets: new Map(),
  placements: new Map(),
  annotations: new Map(),
  placeholders: new Map(),
  selection: new Set(),
  opening: false,
  error: null,
  revealRequest: null,

  openBoard: async (path) => {
    set({ opening: true, error: null });
    try {
      const info = await ipc.openBoard(path);
      const annotations = new Map<string, Shape[]>();
      for (const a of info.doc.annotations ?? []) annotations.set(a.placementId, a.shapes);
      set({
        boardId: info.id,
        folder: info.folder,
        name: info.name,
        assets: indexById(info.doc.assets),
        placements: indexById(info.doc.placements),
        annotations,
        placeholders: new Map(),
        selection: new Set(),
        opening: false,
      });
      useHistoryStore.getState().clear(); // history is per-board
      useUiStore.getState().setTool("select"); // a board always opens on Select
    } catch (e) {
      set({ opening: false, error: String(e) });
    }
  },

  importFiles: async (paths) => {
    const { boardId } = get();
    if (!boardId || paths.length === 0) return [];
    try {
      const result = await ipc.importPaths(boardId, paths);
      set((s) => mergeImport(s, result));
      pushImportUndo(boardId, result.placements, set);
      maybeInstallFfmpegFor(result.assets);
      return result.placements;
    } catch (e) {
      set({ error: String(e) });
      return [];
    }
  },

  importFilesAt: async (paths, center) => {
    const { boardId } = get();
    if (!boardId || paths.length === 0) return [];
    try {
      const result = centerImport(get(), await ipc.importPaths(boardId, paths), center);
      set((s) => mergeImport(s, result));
      pushImportUndo(boardId, result.placements, set);
      maybeInstallFfmpegFor(result.assets);
      const updates = result.placements.map((p) => ({
        id: p.id,
        x: p.x,
        y: p.y,
        scale: p.scale,
        rotation: p.rotation,
        z: p.z,
      }));
      if (updates.length) void ipc.updatePlacements(boardId, updates).catch((e) => set({ error: String(e) }));
      return result.placements;
    } catch (e) {
      set({ error: String(e) });
      return [];
    }
  },

  importBytes: async (bytes, ext, stem) => {
    const { boardId } = get();
    if (!boardId) return [];
    try {
      const result = await ipc.importImageBytes(boardId, Array.from(bytes), ext, stem);
      set((s) => mergeImport(s, result));
      pushImportUndo(boardId, result.placements, set);
      return result.placements;
    } catch (e) {
      set({ error: String(e) });
      return [];
    }
  },

  importBytesAt: async (bytes, ext, stem, center) => {
    const { boardId } = get();
    if (!boardId) return [];
    try {
      const result = centerImport(get(), await ipc.importImageBytes(boardId, Array.from(bytes), ext, stem), center);
      set((s) => mergeImport(s, result));
      pushImportUndo(boardId, result.placements, set);
      const updates = result.placements.map((p) => ({
        id: p.id,
        x: p.x,
        y: p.y,
        scale: p.scale,
        rotation: p.rotation,
        z: p.z,
      }));
      if (updates.length) void ipc.updatePlacements(boardId, updates).catch((e) => set({ error: String(e) }));
      return result.placements;
    } catch (e) {
      set({ error: String(e) });
      return [];
    }
  },

  applyImportResult: (result) => {
    const { boardId } = get();
    if (!boardId) return;
    set((s) => mergeImport(s, result));
    if (result.placements.length > 0) {
      pushImportUndo(boardId, result.placements, set);
    }
    maybeInstallFfmpegFor(result.assets);
  },

  commitMoves: async (updates) => {
    const { boardId, placements } = get();
    if (!boardId || updates.length === 0) return;
    // Capture the prior transforms (the mirror still holds them) for undo.
    const before: PlacementUpdate[] = updates
      .map((u) => placements.get(u.id))
      .filter((p): p is Placement => !!p)
      .map((p) => ({ id: p.id, x: p.x, y: p.y, scale: p.scale, rotation: p.rotation, z: p.z }));
    const apply = (ups: PlacementUpdate[]) => {
      set((s) => {
        const next = new Map(s.placements);
        for (const u of ups) {
          const p = next.get(u.id);
          if (p) next.set(u.id, { ...p, x: u.x, y: u.y, scale: u.scale, rotation: u.rotation, z: u.z });
        }
        return { placements: next };
      });
      void ipc.updatePlacements(boardId, ups).catch((e) => set({ error: String(e) }));
    };
    apply(updates);
    useHistoryStore.getState().push({ label: "Move", undo: () => apply(before), redo: () => apply(updates) });
  },

  setSelection: (ids) => set({ selection: new Set(ids) }),

  addGenerated: (asset, placement, placeholderId) => {
    set((s) => {
      const assets = new Map(s.assets);
      assets.set(asset.id, asset);
      const placements = new Map(s.placements);
      placements.set(placement.id, placement);
      const placeholders = new Map(s.placeholders);
      if (placeholderId) placeholders.delete(placeholderId);
      return { assets, placements, placeholders, selection: new Set([placement.id]) };
    });
    const boardId = get().boardId;
    if (!boardId) return;
    const remove = () => {
      set((s) => {
        const placements = new Map(s.placements);
        placements.delete(placement.id);
        const selection = new Set([...s.selection].filter((id) => id !== placement.id));
        return { placements, selection };
      });
      void ipc.deletePlacements(boardId, [placement.id]).catch((e) => set({ error: String(e) }));
    };
    const add = () => {
      set((s) => {
        const placements = new Map(s.placements);
        placements.set(placement.id, placement);
        const assets = new Map(s.assets);
        assets.set(asset.id, asset);
        return { placements, assets };
      });
      void ipc.restorePlacements(boardId, [placement]).catch((e) => set({ error: String(e) }));
    };
    useHistoryStore.getState().push({ label: "Generate", undo: remove, redo: add });
  },

  addPlaceholder: (id, rect) =>
    set((s) => {
      const placeholders = new Map(s.placeholders);
      placeholders.set(id, rect);
      return { placeholders };
    }),

  clearPlaceholders: () =>
    set((s) => (s.placeholders.size === 0 ? {} : { placeholders: new Map() })),

  revealPlacement: (id) =>
    set((s) => ({
      selection: new Set([id]),
      revealRequest: { id, n: (s.revealRequest?.n ?? 0) + 1 },
    })),

  renameAsset: async (placementId, newName) => {
    const { boardId, placements, assets } = get();
    if (!boardId) return;
    const p = placements.get(placementId);
    const a = p && assets.get(p.assetId);
    if (!a) return;
    const oldName = a.path.split("/").pop() ?? a.path; // full basename (with ext) restores exactly
    const rename = async (name: string) => {
      try {
        const newPath = await ipc.renameAsset(boardId, placementId, name);
        set((s) => {
          const pp = s.placements.get(placementId);
          const aa = pp && s.assets.get(pp.assetId);
          if (!aa) return {};
          const next = new Map(s.assets);
          next.set(aa.id, { ...aa, path: newPath });
          return { assets: next };
        });
      } catch (e) {
        set({ error: String(e) });
      }
    };
    await rename(newName);
    useHistoryStore.getState().push({ label: "Rename", undo: () => rename(oldName), redo: () => rename(newName) });
  },

  setAnnotation: (placementId, shapes) => {
    const { boardId, annotations } = get();
    const before = annotations.get(placementId) ?? [];
    const apply = (sh: Shape[]) => {
      set((s) => {
        const next = new Map(s.annotations);
        if (sh.length) next.set(placementId, sh);
        else next.delete(placementId);
        return { annotations: next };
      });
      if (boardId) void ipc.setAnnotation(boardId, placementId, sh);
    };
    apply(shapes);
    useHistoryStore.getState().push({ label: "Annotate", undo: () => apply(before), redo: () => apply(shapes) });
  },

  consumeMarks: (ids) => {
    const { boardId } = get();
    set((s) => {
      const annotations = new Map(s.annotations);
      let changed = false;
      for (const id of ids)
        if (annotations.has(id)) {
          annotations.delete(id);
          changed = true;
        }
      return changed ? { annotations } : {};
    });
    if (boardId) for (const id of ids) void ipc.setAnnotation(boardId, id, []);
  },

  replacePlacementImage: async (id, bytes) => {
    const { boardId, placements } = get();
    if (!boardId) return;
    const before = placements.get(id);
    try {
      const res = await ipc.replacePlacementImage(boardId, id, bytes, "png");
      const after = res.placement;
      set((s) => {
        const assets = new Map(s.assets);
        if (res.asset) assets.set(res.asset.id, res.asset);
        const next = new Map(s.placements);
        next.set(after.id, after);
        return { assets, placements: next };
      });
      if (before) {
        const apply = (p: Placement) => {
          set((s) => {
            const next = new Map(s.placements);
            next.set(p.id, p);
            return { placements: next };
          });
          void ipc.restorePlacements(boardId, [p]).catch((e) => set({ error: String(e) }));
        };
        useHistoryStore.getState().push({ label: "Crop", undo: () => apply(before), redo: () => apply(after) });
      }
    } catch (e) {
      set({ error: String(e) });
    }
  },

  extractFrame: async (placementId, bytes) => {
    const { boardId } = get();
    if (!boardId) return;
    try {
      const result = await ipc.extractFrame(boardId, placementId, Array.from(bytes));
      set((s) => mergeImport(s, result));
      const newId = result.placements[0]?.id;
      if (newId) set({ selection: new Set([newId]) });
      pushImportUndo(boardId, result.placements, set);
    } catch (e) {
      set({ error: String(e) });
    }
  },

  backfillVideoPosters: async () => {
    const { boardId } = get();
    if (!boardId) return;
    try {
      const updated = await ipc.backfillVideoPosters(boardId);
      if (updated.length === 0) return;
      set((s) => {
        const assets = new Map(s.assets);
        for (const a of updated) assets.set(a.id, a);
        return { assets };
      });
    } catch (e) {
      set({ error: String(e) });
    }
  },

  deleteSelected: async () => {
    const { boardId, selection, placements } = get();
    if (!boardId || selection.size === 0) return;
    const ids = [...selection];
    const deleted = ids.map((id) => placements.get(id)).filter((p): p is Placement => !!p);
    const remove = () => {
      set((s) => {
        const next = new Map(s.placements);
        for (const id of ids) next.delete(id);
        return { placements: next, selection: new Set<string>() };
      });
      void ipc.deletePlacements(boardId, ids).catch((e) => set({ error: String(e) }));
    };
    const restore = () => {
      set((s) => {
        const next = new Map(s.placements);
        for (const p of deleted) next.set(p.id, p);
        return { placements: next };
      });
      void ipc.restorePlacements(boardId, deleted).catch((e) => set({ error: String(e) }));
    };
    remove();
    useHistoryStore.getState().push({ label: "Delete", undo: restore, redo: remove });
  },
}));
