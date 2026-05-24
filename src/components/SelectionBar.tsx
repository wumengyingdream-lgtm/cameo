import { useState, type RefObject } from "react";
import { Eraser, Sparkles, Crop, MoreHorizontal, Copy, FolderOpen, Download, Trash2 } from "lucide-react";
import { useBoardStore } from "../store/board";
import { useChatStore } from "../store/chat";
import { useUiStore } from "../store/ui";
import { ipc } from "../lib/ipc";
import { PRESET_REMOVE_BG, PRESET_UPSCALE, runImagePreset, exportImage } from "../lib/imageActions";
import { useT } from "../i18n/locale";

/** Per-image action bar — floats just above the selected image's title (the
 *  scene positions `rootRef` every frame so it follows drag/pan/zoom). Shown
 *  only for a single selection. The same actions are also reachable via the
 *  canvas right-click menu (see CanvasContextMenu, sharing lib/imageActions). */
export function SelectionBar({ rootRef }: { rootRef: RefObject<HTMLDivElement | null> }) {
  const selection = useBoardStore((s) => s.selection);
  const boardId = useBoardStore((s) => s.boardId);
  const ready = useChatStore((s) => s.sessionStatus === "ready" && s.turnStatus !== "running");
  const [more, setMore] = useState(false);
  const t = useT();

  const first = selection.size === 1 ? [...selection][0] : null;

  const runPreset = (prompt: string) => {
    if (!ready || !first || !boardId) return;
    runImagePreset(boardId, first, prompt);
  };

  return (
    <div className="cm-selbar" ref={rootRef} style={{ display: "none" }}>
      {first && boardId && (
        <>
          <button className="cm-selbar__btn" disabled={!ready} title={t("img.removeBgTitle")} onClick={() => runPreset(PRESET_REMOVE_BG)}>
            <Eraser size={14} />
            {t("img.removeBg")}
          </button>
          <button className="cm-selbar__btn" disabled={!ready} title={t("img.upscaleTitle")} onClick={() => runPreset(PRESET_UPSCALE)}>
            <Sparkles size={14} />
            {t("img.upscale")}
          </button>
          <button className="cm-selbar__btn" title={t("img.crop")} onClick={() => useUiStore.getState().setCropping(first)}>
            <Crop size={14} />
            {t("img.crop")}
          </button>
          <div className="cm-selbar__more">
            <button className="cm-selbar__btn" title={t("img.more")} onClick={() => setMore((m) => !m)}>
              <MoreHorizontal size={15} />
            </button>
            {more && (
              <div className="cm-selbar__menu" onMouseLeave={() => setMore(false)}>
                <button onClick={() => { setMore(false); void ipc.copyImage(boardId, first); }}>
                  <Copy size={14} />
                  {t("img.copy")}
                </button>
                <button onClick={() => { setMore(false); void ipc.revealInFinder(boardId, first); }}>
                  <FolderOpen size={14} />
                  {t("img.reveal")}
                </button>
                <button onClick={() => { setMore(false); void exportImage(boardId, first); }}>
                  <Download size={14} />
                  {t("img.export")}
                </button>
                <button className="cm-selbar__del" onClick={() => { setMore(false); void useBoardStore.getState().deleteSelected(); }}>
                  <Trash2 size={14} />
                  {t("img.delete")}
                </button>
              </div>
            )}
          </div>
        </>
      )}
    </div>
  );
}
