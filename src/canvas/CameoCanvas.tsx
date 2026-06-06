import { useCallback, useEffect, useRef, useState } from "react";
import { CanvasScene, type CanvasContextTarget } from "./scene";
import { useUiStore } from "../store/ui";
import { useBoardStore } from "../store/board";
import { useClipboardStore, CAMEO_CLIP_MARKER } from "../store/clipboard";
import { useVideoPlaybackStore } from "../store/videoPlayback";
import { useHistoryStore } from "../store/history";
import { useWorkspaceStore } from "../store/workspace";
import type { ClipItem } from "../types";
import { SelectionBar } from "../components/SelectionBar";
import { CropOverlay } from "../components/CropOverlay";
import { VideoOverlay } from "./VideoOverlay";
import { CanvasContextMenu } from "../components/CanvasContextMenu";
import { isVideoAsset } from "../lib/media";
import { copyImageToClipboard } from "../lib/imageActions";
import { useT } from "../i18n/locale";
import { useFileImport } from "../lib/useFileImport";

/**
 * Mounts the PixiJS scene and bridges it to the stores:
 * - pushes board data (placements/assets) into the scene on change,
 * - relays scene gestures (selection, moves) back into the store,
 * - handles the Delete key for the current selection.
 *
 * React renders this once; the canvas is driven imperatively.
 */
export function CameoCanvas() {
  const hostRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<CanvasScene | null>(null);
  const barRef = useRef<HTMLDivElement>(null);
  const cropRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLDivElement>(null);
  const [ctxMenu, setCtxMenu] = useState<CanvasContextTarget | null>(null);
  const chatOpen = useUiStore((s) => s.chatOpen);
  const chatWidth = useUiStore((s) => s.chatWidth);
  const sidebarOpen = useWorkspaceStore((s) => s.sidebarOpen);
  const t = useT();
  const markComment = t("canvas.markComment");
  const renameHint = t("canvas.renameHint");

  const importAtDropPoint = useCallback(async (paths: string[], position?: { x: number; y: number }) => {
    if (!position || !sceneRef.current) {
      await useBoardStore.getState().importFiles(paths);
      return;
    }
    await useBoardStore.getState().importFilesAt(paths, sceneRef.current.screenToWorldPoint(position.x, position.y));
  }, []);

  useFileImport({ onDrop: importAtDropPoint });

  // Scene lifecycle.
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const scene = new CanvasScene();
    sceneRef.current = scene;
    let alive = true;

    void scene.init(host, {
      onStats: (s) => alive && useUiStore.getState().setStats(s),
      onSelectionChange: (ids) => alive && useBoardStore.getState().setSelection(ids),
      onSpacePanChange: (active) => alive && useUiStore.getState().setSpaceHand(active),
      onSpacebar: () => {
        if (!alive) return false;
        // A focused on-canvas video gets spacebar for play/pause (it registered
        // its controls); otherwise let the scene use space for transient pan.
        const pb = useVideoPlaybackStore.getState();
        if (pb.placementId && pb.toggle) {
          pb.toggle();
          return true;
        }
        return false;
      },
      onCommitMoves: (u) => alive && void useBoardStore.getState().commitMoves(u),
      onCommitTextNodes: (nodes) => {
        if (!alive) return;
        for (const node of nodes) void useBoardStore.getState().updateTextNode(node);
      },
      onCreateText: (at) => alive && void useBoardStore.getState().addTextNodeAt(at),
      onCreateLine: (at) => alive && void useBoardStore.getState().addLineNodeAt(at),
      onAnnotate: (id, shapes) => alive && useBoardStore.getState().setAnnotation(id, shapes),
      onRename: (id, name) => alive && void useBoardStore.getState().renameAsset(id, name),
      onContextMenu: (t) => alive && setCtxMenu(t),
      onSelRect: (rect) => {
        const bar = barRef.current;
        const crop = cropRef.current;
        const video = videoRef.current;
        const cropping = useUiStore.getState().cropping;
        // On-canvas video player tracks the focused video's image area. Only a
        // single video selection (and not while cropping) gets the live <video>;
        // the scene drives its rect every frame, React owns content.
        if (video) {
          const bs = useBoardStore.getState();
          const selId = bs.selection.size === 1 ? [...bs.selection][0] : null;
          const p = selId ? bs.placements.get(selId) : null;
          const a = p ? bs.assets.get(p.assetId) ?? null : null;
          if (rect && !cropping && isVideoAsset(a)) {
            video.style.display = "block";
            video.style.left = `${rect.imageX}px`;
            video.style.top = `${rect.imageY}px`;
            video.style.width = `${rect.imageW}px`;
            video.style.height = `${rect.imageH}px`;
            video.style.setProperty("--cm-vid-rotation", `${rect.rotation}rad`);
          } else {
            video.style.display = "none";
          }
        }
        if (bar) {
          if (rect && !cropping) {
            bar.style.display = "flex";
            bar.style.left = `${rect.x + rect.w / 2}px`;
            bar.style.top = `${rect.y}px`;
          } else {
            bar.style.display = "none";
          }
        }
        if (crop) {
          if (rect && cropping) {
            crop.style.display = "block";
            crop.style.left = `${rect.x}px`;
            crop.style.top = `${rect.y}px`;
            crop.style.width = `${rect.w}px`;
            crop.style.height = `${rect.h}px`;
            crop.style.setProperty("--cm-crop-area-left", `${rect.imageX - rect.x}px`);
            crop.style.setProperty("--cm-crop-area-top", `${rect.imageY - rect.y}px`);
            crop.style.setProperty("--cm-crop-area-width", `${rect.imageW}px`);
            crop.style.setProperty("--cm-crop-area-height", `${rect.imageH}px`);
            crop.style.setProperty("--cm-crop-area-rotation", `${rect.rotation}rad`);
          } else {
            crop.style.display = "none";
          }
        }
      },
    });
    scene.setTool(useUiStore.getState().tool);
    useUiStore.getState().setCanvasZoom((direction) => {
      if (direction === "reset") scene.resetZoom();
      else scene.zoomStep(direction);
    });

    return () => {
      alive = false;
      useUiStore.getState().setCanvasZoom(null);
      scene.destroy();
      sceneRef.current = null;
    };
  }, []);

  // Push board data + selection into the scene on change (reference compare —
  // the store hands out fresh Maps/Sets on mutation). Pushing selection is safe:
  // scene.setSelection only redraws, it never re-fires onSelectionChange.
  useEffect(() => {
    const pushData = (s = useBoardStore.getState()) =>
      sceneRef.current?.setData(s.boardId, s.placements, s.assets, s.textNodes, s.annotations, s.placeholders);
    const pushSel = (s = useBoardStore.getState()) =>
      sceneRef.current?.setSelection([...s.selection]);
    pushData();
    pushSel();
    const unsubBoard = useBoardStore.subscribe((state, prev) => {
      if (
        state.boardId !== prev.boardId ||
        state.placements !== prev.placements ||
        state.assets !== prev.assets ||
        state.textNodes !== prev.textNodes ||
        state.annotations !== prev.annotations ||
        state.placeholders !== prev.placeholders
      ) {
        pushData(state);
      }
      if (state.selection !== prev.selection) pushSel(state);
      if (state.revealRequest !== prev.revealRequest && state.revealRequest) {
        sceneRef.current?.focusPlacement(state.revealRequest.id);
      }
    });
    const unsubTool = useUiStore.subscribe((state, prev) => {
      if (state.tool !== prev.tool) sceneRef.current?.setTool(state.tool);
      if (state.cropping !== prev.cropping) sceneRef.current?.setCropActive(!!state.cropping);
      if (state.minimapVisible !== prev.minimapVisible) sceneRef.current?.setMinimapVisible(state.minimapVisible);
    });
    return () => {
      unsubBoard();
      unsubTool();
    };
  }, []);

  // Delete key removes the current selection.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const target = e.target as HTMLElement | null;
      if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.isContentEditable)) {
        return;
      }
      if (e.key === "Escape" && useUiStore.getState().cropping) {
        e.preventDefault();
        useUiStore.getState().setCropping(null);
        return;
      }
      if (
        useUiStore.getState().cropping ||
        document.querySelector(".cm-ctx, .cm-modal-backdrop, .cm-gallery-backdrop, .cm-gdetail-backdrop, .cm-compare")
      ) {
        return;
      }
      if ((e.metaKey || e.ctrlKey) && (e.key === "z" || e.key === "Z")) {
        e.preventDefault();
        if (e.shiftKey) useHistoryStore.getState().redo();
        else useHistoryStore.getState().undo();
        return;
      }
      // Navigation: ⇧1 fit all, ⇧2 zoom to selection, ⌘0 reset 100%.
      if (e.shiftKey && e.code === "Digit1") {
        e.preventDefault();
        sceneRef.current?.fitAll();
        return;
      }
      if (e.shiftKey && e.code === "Digit2") {
        e.preventDefault();
        sceneRef.current?.fitSelection();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && (e.code === "Equal" || e.code === "NumpadAdd" || e.key === "+" || e.key === "=")) {
        e.preventDefault();
        sceneRef.current?.zoomStep("in");
        return;
      }
      if ((e.metaKey || e.ctrlKey) && (e.code === "Minus" || e.code === "NumpadSubtract" || e.key === "-")) {
        e.preventDefault();
        sceneRef.current?.zoomStep("out");
        return;
      }
      if ((e.metaKey || e.ctrlKey) && e.code === "Digit0") {
        e.preventDefault();
        sceneRef.current?.resetZoom();
        return;
      }
      if ((e.metaKey || e.ctrlKey) && (e.key === "c" || e.key === "C")) {
        const b = useBoardStore.getState();
        if (!b.boardId || b.selection.size === 0) return;
        e.preventDefault();
        const selectedPlacementIds = [...b.selection].filter((id) => b.placements.has(id));
        const selectedTextIds = [...b.selection].filter((id) => b.textNodes.has(id));
        if (selectedTextIds.length > 0 && selectedPlacementIds.length === 0) {
          void b.duplicateTextNodes(selectedTextIds);
          return;
        }
        if (selectedPlacementIds.length === 1) {
          void copyImageToClipboard(b.boardId, selectedPlacementIds[0]);
          return;
        }
        // Copy the WHOLE selection (images + videos) into the in-app clipboard
        // by source path + transform, so paste re-imports all of them — even
        // across boards. The OS clipboard separately gets the first image for
        // external-app interop (videos can't rasterize to a bitmap).
        const items: ClipItem[] = [];
        for (const id of b.selection) {
          const p = b.placements.get(id);
          const a = p && b.assets.get(p.assetId);
          if (!p || !a) continue;
          items.push({ assetPath: a.path, x: p.x, y: p.y, scale: p.scale, rotation: p.rotation, crop: p.crop });
        }
        if (items.length) {
          useClipboardStore.getState().set(b.boardId, items);
          // Stamp the OS clipboard so paste can tell this in-app copy is the most
          // recent one (vs. an image copied elsewhere afterwards). Track success:
          // if writeText is unavailable/denied, paste falls back to the in-app
          // store instead of silently dropping the copy. External image interop
          // lives on the explicit "Copy image" action in SelectionBar.
          navigator.clipboard
            .writeText(CAMEO_CLIP_MARKER)
            .then(() => useClipboardStore.getState().setMarked(true))
            .catch(() => useClipboardStore.getState().setMarked(false));
        }
        return;
      }
      if (e.key === "Delete" || e.key === "Backspace") {
        if (useBoardStore.getState().selection.size > 0) {
          e.preventDefault();
          void useBoardStore.getState().deleteSelected();
        }
      } else if (e.metaKey || e.ctrlKey) {
        // Bare-letter tool shortcuts must not fire for Cmd/Ctrl combos
        // (e.g. Cmd+V paste would otherwise also switch to the Select tool).
      } else if (e.key === "v" || e.key === "V") {
        useUiStore.getState().setTool("select");
      } else if (e.key === "h" || e.key === "H") {
        useUiStore.getState().setTool("hand");
      } else if (e.key === "t" || e.key === "T") {
        useUiStore.getState().setTool("text");
      } else if (e.key === "l" || e.key === "L") {
        useUiStore.getState().setTool("line");
      } else if (e.key === "r" || e.key === "R") {
        useUiStore.getState().setTool("rect");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Keep fit / reveal clear of the floating chrome so outputs land in view.
  useEffect(() => {
    sceneRef.current?.setSafeInsets({
      left: sidebarOpen ? 252 : 0,
      right: chatOpen ? chatWidth + 20 : 0,
      top: 56,
      bottom: 72,
    });
  }, [chatOpen, chatWidth, sidebarOpen]);

  // Push localized canvas-overlay strings to the scene (re-applies on lang change).
  useEffect(() => {
    sceneRef.current?.setStrings({ markComment, renameHint });
  }, [markComment, renameHint]);

  return (
    <>
      <div ref={hostRef} className="cm-canvas-host" />
      <SelectionBar rootRef={barRef} />
      <CropOverlay rootRef={cropRef} />
      <VideoOverlay rootRef={videoRef} />
      <CanvasContextMenu
        menu={ctxMenu}
        onClose={() => setCtxMenu(null)}
        onFitAll={() => sceneRef.current?.fitAll()}
      />
    </>
  );
}
