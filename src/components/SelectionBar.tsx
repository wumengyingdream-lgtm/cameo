import { useState, type RefObject } from "react";
import { Eraser, Sparkles, Crop, MoreHorizontal, Copy, FolderOpen, Download, Trash2, AtSign } from "lucide-react";
import { useBoardStore } from "../store/board";
import { useChatStore } from "../store/chat";
import { useUiStore } from "../store/ui";
import { useComposerStore } from "../store/composer";
import { useVideoPlaybackStore } from "../store/videoPlayback";
import { ipc } from "../lib/ipc";
import { isVideoAsset } from "../lib/media";
import { PRESET_REMOVE_BG, PRESET_UPSCALE, runImagePreset, exportImage } from "../lib/imageActions";
import { useT } from "../i18n/locale";

/** "M:SS" for a video position in seconds (the frame-reference pill badge). */
function fmtClock(s: number): string {
  const m = Math.floor(s / 60);
  const ss = Math.floor(s % 60);
  return `${m}:${ss.toString().padStart(2, "0")}`;
}

/** Per-image action bar — floats just above the selected image's title (the
 *  scene positions `rootRef` every frame so it follows drag/pan/zoom). Shown
 *  only for a single selection. The same actions are also reachable via the
 *  canvas right-click menu (see CanvasContextMenu, sharing lib/imageActions).
 *
 *  For a video selection the image-only actions (remove-bg / upscale / crop /
 *  copy-image / export) are gated off — those are overlay-as-image / pixel ops
 *  with no per-frame meaning. Video editing flows through the chat (Codex ×
 *  ffmpeg); the canvas keeps reveal + delete (scrub lives in VideoOverlay). */
export function SelectionBar({ rootRef }: { rootRef: RefObject<HTMLDivElement | null> }) {
  const selection = useBoardStore((s) => s.selection);
  const assets = useBoardStore((s) => s.assets);
  const placements = useBoardStore((s) => s.placements);
  const boardId = useBoardStore((s) => s.boardId);
  const ready = useChatStore((s) => s.sessionStatus === "ready" && s.turnStatus !== "running");
  const [more, setMore] = useState(false);
  const t = useT();

  const first = selection.size === 1 ? [...selection][0] : null;
  const firstPlacement = first ? placements.get(first) : null;
  const firstAsset = firstPlacement ? assets.get(firstPlacement.assetId) : null;
  const isVideo = isVideoAsset(firstAsset);

  const runPreset = (prompt: string) => {
    if (!ready || !first || !boardId) return;
    runImagePreset(boardId, first, prompt);
  };

  /** "Reference" the selected video into the composer (PRD §17/F6): the whole
   *  file at its start, or — if the user has scrubbed to a frame — that exact
   *  frame (still + timestamp). Falls back to a whole-file reference if frame
   *  extraction fails (ffmpeg unavailable). */
  const referenceVideo = async () => {
    if (!boardId || !first) return;
    const pb = useVideoPlaybackStore.getState();
    const atScrubbedFrame = pb.placementId === first && pb.scrubbed && pb.currentTime > 0.05;
    if (atScrubbedFrame) {
      try {
        const path = await ipc.referenceVideoFrame(boardId, first, pb.currentTime);
        useComposerStore.getState().injectPill(first, {
          atMs: Math.round(pb.currentTime * 1000),
          path,
          label: fmtClock(pb.currentTime),
        });
        return;
      } catch {
        // ffmpeg missing / extraction failed → whole-file reference instead.
      }
    }
    useComposerStore.getState().injectPill(first);
  };

  return (
    <div className="cm-selbar" ref={rootRef} style={{ display: "none" }}>
      {first && boardId && firstPlacement && (
        <>
          {isVideo && (
            <button className="cm-selbar__btn" title={t("video.referenceTitle")} onClick={() => void referenceVideo()}>
              <AtSign size={14} />
              {t("video.reference")}
            </button>
          )}
          {!isVideo && (
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
            </>
          )}
          <div className="cm-selbar__more">
            <button className="cm-selbar__btn" title={t("img.more")} onClick={() => setMore((m) => !m)}>
              <MoreHorizontal size={15} />
            </button>
            {more && (
              <div className="cm-selbar__menu" onMouseLeave={() => setMore(false)}>
                {!isVideo && (
                  <button onClick={() => { setMore(false); void ipc.copyImage(boardId, first); }}>
                    <Copy size={14} />
                    {t("img.copy")}
                  </button>
                )}
                <button onClick={() => { setMore(false); void ipc.revealInFinder(boardId, first); }}>
                  <FolderOpen size={14} />
                  {t("img.reveal")}
                </button>
                {!isVideo && (
                  <button onClick={() => { setMore(false); void exportImage(boardId, first); }}>
                    <Download size={14} />
                    {t("img.export")}
                  </button>
                )}
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
