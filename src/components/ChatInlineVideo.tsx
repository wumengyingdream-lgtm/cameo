import { useEffect, useRef, useState } from "react";
import { FileText, FolderOpen, Film, ImagePlus, Maximize, Play } from "lucide-react";
import { useBoardStore } from "../store/board";
import { useComposerStore } from "../store/composer";
import { cameoUrl, ipc } from "../lib/ipc";
import type { ChatImageResolution } from "../lib/ipc";
import { useT } from "../i18n/locale";

/**
 * Renders a video path the AI emitted in chat text as a small inline player.
 * The video modality's chat counterpart to ChatInlineImage: width-constrained
 * like an image thumbnail, and — like the canvas — it shows the FIRST FRAME as
 * a still by default (no autoplay, no always-on chrome) with a play overlay;
 * clicking starts playback and reveals native controls. In-workspace videos
 * load through the Cameo image protocol (which serves Range requests, so the
 * player can seek); out-of-workspace videos show a chip (no protocol reach).
 * Right-click bridges the artifact back into the canvas/reference loop.
 *
 * The first-frame still comes from the `poster` attribute (the same
 * content-addressed JPEG the canvas renders, resolved by the backend), NOT from
 * `preload="metadata"` — a bare `<video>` paints blank until played in
 * WKWebView, so the poster is what makes the still appear.
 */
export function ChatInlineVideo({ res, basename }: { res: ChatImageResolution; basename: string }) {
  const t = useT();
  const boardId = useBoardStore((s) => s.boardId);
  const placements = useBoardStore((s) => s.placements);
  const assets = useBoardStore((s) => s.assets);
  const videoRef = useRef<HTMLVideoElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const [addedPlacementId, setAddedPlacementId] = useState<string | null>(null);
  // Click-to-play: until the user starts it, the element shows its first-frame
  // poster (see `poster` below) with a play badge and no chrome.
  const [started, setStarted] = useState(false);

  const onContext = (e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    setMenu({ x: e.clientX, y: e.clientY });
  };

  const start = () => {
    setStarted(true);
    void videoRef.current?.play().catch(() => {});
  };

  const existingPlacementId = addedPlacementId ?? res.existingPlacementId ?? null;
  const existingPlacement = existingPlacementId ? placements.get(existingPlacementId) : null;
  const existingAsset = existingPlacement ? assets.get(existingPlacement.assetId) : null;

  const src =
    res.inWorkspace && boardId && res.workspaceRelPath
      ? cameoUrl(boardId, res.workspaceRelPath)
      : "";

  // First-frame still: the backend extracts (and shares with the canvas) a
  // content-addressed poster for in-workspace videos. Without it the <video>
  // is blank until played in WKWebView. Fall back to the on-canvas asset's
  // posterPath so a poster the canvas backfilled AFTER a late ffmpeg install
  // shows in chat too (reactive — no cache to invalidate); same JPEG either way.
  const posterRel = res.posterRelPath ?? existingAsset?.posterPath ?? null;
  const poster = res.inWorkspace && boardId && posterRel ? cameoUrl(boardId, posterRel) : undefined;

  return (
    <span className="cm-chatvid-wrap">
      {src ? (
        <span
          className="cm-chatvid-frame"
          onContextMenu={onContext}
          onClick={() => { if (!started) start(); }}
          role="button"
          aria-label={t("vid.play")}
          title={basename}
        >
          <video
            ref={videoRef}
            className="cm-chatvid"
            src={src}
            poster={poster}
            controls={started}
            preload="metadata"
            muted
            playsInline
            onPlay={() => setStarted(true)}
          />
          {!started && (
            <span className="cm-chatvid-play" aria-hidden="true">
              <Play size={20} fill="currentColor" strokeWidth={0} />
            </span>
          )}
        </span>
      ) : (
        // Out-of-workspace: no protocol reach. Offer the chip + "add to canvas".
        <span className="cm-chatimg cm-chatimg--missing" onContextMenu={onContext} title={res.absPath}>
          <Film size={13} />
          <span className="cm-chatimg__name">{basename}</span>
        </span>
      )}
      {menu && (
        <ChatVideoMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          absPath={res.absPath}
          existingPlacementId={existingPlacement ? existingPlacementId : null}
          onAddedPlacement={setAddedPlacementId}
        />
      )}
    </span>
  );
}

interface MenuProps {
  x: number;
  y: number;
  onClose: () => void;
  absPath: string;
  existingPlacementId: string | null;
  onAddedPlacement: (id: string) => void;
}

function ChatVideoMenu({ x, y, onClose, absPath, existingPlacementId, onAddedPlacement }: MenuProps) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);

  const doShowOnCanvas = () => {
    if (existingPlacementId) useBoardStore.getState().revealPlacement(existingPlacementId);
    onClose();
  };

  const doAddToCanvas = async () => {
    const boardId = useBoardStore.getState().boardId;
    if (!boardId) return onClose();
    try {
      const result = await ipc.importChatImageToCanvas(boardId, absPath);
      const placement = result.placements[0];
      if (placement) {
        useBoardStore.getState().applyImportResult(result);
        onAddedPlacement(placement.id);
      }
    } catch {
      /* silent */
    }
    onClose();
  };

  const doUseAsRef = async () => {
    let placementId = existingPlacementId;
    if (!placementId) {
      const boardId = useBoardStore.getState().boardId;
      if (!boardId) return onClose();
      try {
        const result = await ipc.importChatImageToCanvas(boardId, absPath);
        useBoardStore.getState().applyImportResult(result);
        placementId = result.placements[0]?.id ?? null;
        if (placementId) onAddedPlacement(placementId);
      } catch {
        return onClose();
      }
    }
    if (placementId) useComposerStore.getState().injectPill(placementId);
    onClose();
  };

  const doCopyPath = () => {
    void navigator.clipboard.writeText(absPath).catch(() => {});
    onClose();
  };

  const doReveal = () => {
    void ipc.revealPathInFinder(absPath).catch(() => {});
    onClose();
  };

  return (
    <div ref={ref} className="cm-ctx cm-ctx--chatimg" style={{ left: x, top: y }} role="menu">
      {existingPlacementId ? (
        <button className="cm-ctx__item" onClick={doShowOnCanvas} role="menuitem">
          <Maximize size={14} />
          {t("chatImg.showOnCanvas")}
        </button>
      ) : (
        <button className="cm-ctx__item" onClick={() => void doAddToCanvas()} role="menuitem">
          <ImagePlus size={14} />
          {t("chatImg.addToCanvas")}
        </button>
      )}
      <button className="cm-ctx__item" onClick={() => void doUseAsRef()} role="menuitem">
        <Film size={14} />
        {t("chatImg.useAsRef")}
      </button>
      <div className="cm-ctx__sep" />
      <button className="cm-ctx__item" onClick={doCopyPath} role="menuitem">
        <FileText size={14} />
        {t("chatImg.copyPath")}
      </button>
      <button className="cm-ctx__item" onClick={doReveal} role="menuitem">
        <FolderOpen size={14} />
        {t("chatImg.reveal")}
      </button>
    </div>
  );
}
