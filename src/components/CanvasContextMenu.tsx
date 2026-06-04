import { useEffect } from "react";
import { Eraser, Sparkles, Crop, Copy, FolderOpen, Download, Trash2, ClipboardPaste, Maximize } from "lucide-react";
import type { CanvasContextTarget } from "../canvas/scene";
import { useBoardStore } from "../store/board";
import { useChatStore } from "../store/chat";
import { useUiStore } from "../store/ui";
import { ipc } from "../lib/ipc";
import { isVideoAsset } from "../lib/media";
import { PRESET_REMOVE_BG, PRESET_UPSCALE, runImagePreset, exportImages, copyImageToClipboard } from "../lib/imageActions";
import { useT } from "../i18n/locale";

/** Native-style right-click menu on the canvas. On an image it flattens the
 *  selection-bar actions + the "更多" menu; on empty space it offers paste +
 *  fit-all. Shares lib/imageActions with the floating SelectionBar. */
export function CanvasContextMenu({
  menu,
  onClose,
  onFitAll,
}: {
  menu: CanvasContextTarget | null;
  onClose: () => void;
  onFitAll: () => void;
}) {
  const t = useT();
  useEffect(() => {
    if (!menu) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    // Non-capture so the menu's own onPointerDown (stopPropagation) can keep it
    // open; clicks/scroll anywhere else dismiss it.
    window.addEventListener("pointerdown", onClose);
    window.addEventListener("wheel", onClose);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onClose);
      window.removeEventListener("wheel", onClose);
      window.removeEventListener("keydown", onKey);
    };
  }, [menu, onClose]);

  if (!menu) return null;

  const board = useBoardStore.getState();
  const boardId = board.boardId;
  const chat = useChatStore.getState();
  const ready = chat.sessionStatus === "ready" && chat.turnStatus !== "running";

  // Run an action then close.
  const act = (fn: () => void) => () => {
    onClose();
    fn();
  };

  // Clamp to the viewport (rough height estimate per menu kind).
  const selectedIds = [...board.selection];
  const selectedPlacementIds = selectedIds.filter((id) => board.placements.has(id));
  const multiSelectImage =
    menu.kind === "image" && selectedPlacementIds.length > 1 && board.selection.has(menu.placementId);
  const exportIds = menu.kind === "image" && multiSelectImage ? selectedPlacementIds : menu.kind === "image" ? [menu.placementId] : [];
  const estH = menu.kind === "image" ? (multiSelectImage ? 192 : 320) : menu.kind === "text" ? 56 : 96;
  const left = Math.max(8, Math.min(menu.x, window.innerWidth - 208));
  const top = Math.max(8, Math.min(menu.y, window.innerHeight - estH - 8));
  const style = { left, top } as const;

  if (menu.kind === "image") {
    const pid = menu.placementId;
    // Image-only actions (remove-bg / upscale / crop / copy-image / export) are
    // pixel/overlay-as-image ops with no per-frame meaning for video — and
    // copy_image/bakeCrop would try to image::open(.mp4) and fail. Gate them off
    // for a video placement, matching SelectionBar (W1). Reveal + delete remain.
    const placement = board.placements.get(pid);
    const asset = placement ? board.assets.get(placement.assetId) : undefined;
    const isVideo = isVideoAsset(asset);
    return (
      <div className="cm-ctx" style={style} onPointerDown={(e) => e.stopPropagation()}>
        {!multiSelectImage && !isVideo && (
          <>
            <button className="cm-ctx__item" disabled={!ready} onClick={act(() => boardId && runImagePreset(boardId, pid, PRESET_REMOVE_BG))}>
              <Eraser size={14} />
              {t("img.removeBg")}
            </button>
            <button className="cm-ctx__item" disabled={!ready} onClick={act(() => boardId && runImagePreset(boardId, pid, PRESET_UPSCALE))}>
              <Sparkles size={14} />
              {t("img.upscale")}
            </button>
            <button className="cm-ctx__item" onClick={act(() => useUiStore.getState().setCropping(pid))}>
              <Crop size={14} />
              {t("img.crop")}
            </button>
            <div className="cm-ctx__sep" />
          </>
        )}
        {!isVideo && (
          <button className="cm-ctx__item" onClick={act(() => boardId && void copyImageToClipboard(boardId, pid))}>
            <Copy size={14} />
            {t("img.copy")}
          </button>
        )}
        <button className="cm-ctx__item" onClick={act(() => boardId && void ipc.revealInFinder(boardId, pid))}>
          <FolderOpen size={14} />
          {t("img.reveal")}
        </button>
        {!isVideo && (
          <button className="cm-ctx__item" onClick={act(() => boardId && void exportImages(boardId, exportIds))}>
            <Download size={14} />
            {multiSelectImage ? t("img.exportSelected", { count: exportIds.length }) : t("img.export")}
          </button>
        )}
        <div className="cm-ctx__sep" />
        <button className="cm-ctx__item cm-ctx__item--danger" onClick={act(() => void useBoardStore.getState().deleteSelected())}>
          <Trash2 size={14} />
          {t("img.delete")}
        </button>
      </div>
    );
  }

  if (menu.kind === "text") {
    return (
      <div className="cm-ctx" style={style} onPointerDown={(e) => e.stopPropagation()}>
        <button className="cm-ctx__item cm-ctx__item--danger" onClick={act(() => void useBoardStore.getState().deleteSelected())}>
          <Trash2 size={14} />
          {t("img.delete")}
        </button>
      </div>
    );
  }

  // Empty canvas: paste an image at the click, or fit everything into view.
  const worldX = menu.worldX;
  const worldY = menu.worldY;
  const paste = async () => {
    const bytes = await ipc.readClipboardImage();
    if (!bytes || !boardId) return;
    await useBoardStore.getState().importBytesAt(new Uint8Array(bytes), "png", "pasted", { x: worldX, y: worldY });
  };

  return (
    <div className="cm-ctx" style={style} onPointerDown={(e) => e.stopPropagation()}>
      <button className="cm-ctx__item" onClick={act(() => void paste())}>
        <ClipboardPaste size={14} />
        {t("ctx.paste")}
      </button>
      <button className="cm-ctx__item" onClick={act(onFitAll)}>
        <Maximize size={14} />
        {t("ctx.fitAll")}
      </button>
    </div>
  );
}
