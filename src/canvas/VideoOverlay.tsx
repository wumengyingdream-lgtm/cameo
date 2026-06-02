import { useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { Play, Pause, Volume2, VolumeX, ChevronLeft, ChevronRight, ImagePlus } from "lucide-react";
import { useBoardStore } from "../store/board";
import { useUiStore } from "../store/ui";
import { cameoUrl, ipc } from "../lib/ipc";
import { isVideoAsset } from "../lib/media";
import { useT } from "../i18n/locale";

/**
 * The on-canvas video player. Like CropOverlay/SelectionBar, the PixiJS scene
 * positions `rootRef` every frame (image rect in screen space) so the player
 * tracks pan/zoom/drag — React only owns the content. Shown for a single video
 * selection that isn't being cropped.
 *
 * Design (PRD §6.5, decision E2):
 *  - The `<video>` is `pointer-events: none`, so canvas drag/select/delete keep
 *    working *through* it (the poster sprite underneath is the hit target). The
 *    control bar alone captures pointer.
 *  - Only the focused video gets a live `<video>`; every other video on the
 *    board stays a static poster sprite — N videos cost ~N images, not N decode
 *    pipelines (avoids the SP-3 perf cliff).
 *  - Scrub seeks the `cameo://` source, which serves HTTP Range (206) — without
 *    that the element refuses to seek. crossOrigin lets extractFrame read pixels.
 */
export function VideoOverlay({ rootRef }: { rootRef: RefObject<HTMLDivElement | null> }) {
  const selection = useBoardStore((s) => s.selection);
  const placements = useBoardStore((s) => s.placements);
  const assets = useBoardStore((s) => s.assets);
  const boardId = useBoardStore((s) => s.boardId);
  const cropping = useUiStore((s) => s.cropping);
  const t = useT();

  const videoRef = useRef<HTMLVideoElement>(null);
  const [playing, setPlaying] = useState(false);
  const [muted, setMuted] = useState(true);
  const [cur, setCur] = useState(0);
  const [dur, setDur] = useState(0);

  const sel = selection.size === 1 ? [...selection][0] : null;
  const placement = sel ? placements.get(sel) : null;
  const asset = placement ? assets.get(placement.assetId) ?? null : null;
  const isVideo = isVideoAsset(asset);
  const show = !!sel && isVideo && !cropping && !!boardId && !!asset;

  // Stable src — only changes when the asset changes (NOT per frame).
  const src = useMemo(
    () => (show && boardId && asset ? cameoUrl(boardId, asset.path) : ""),
    [show, boardId, asset],
  );
  const fps = asset?.fps && asset.fps > 0 ? asset.fps : 30;

  // Reset transient playback state when the selected video changes.
  useEffect(() => {
    setPlaying(false);
    setCur(0);
    setDur(0);
  }, [src]);

  const togglePlay = () => {
    const v = videoRef.current;
    if (!v) return;
    if (v.paused) void v.play();
    else v.pause();
  };

  const stepFrame = (dir: 1 | -1) => {
    const v = videoRef.current;
    if (!v) return;
    v.pause();
    const next = Math.max(0, Math.min(v.duration || dur, v.currentTime + dir / fps));
    v.currentTime = next;
  };

  const onScrub = (e: React.ChangeEvent<HTMLInputElement>) => {
    const v = videoRef.current;
    if (!v) return;
    const time = Number(e.target.value);
    // fastSeek where available keeps dragging smooth; assignment is the precise
    // fallback (and what fires on release).
    if (typeof v.fastSeek === "function") v.fastSeek(time);
    else v.currentTime = time;
    setCur(time);
  };

  const toggleMute = () => {
    const v = videoRef.current;
    if (!v) return;
    v.muted = !v.muted;
    setMuted(v.muted);
  };

  const extractFrame = async () => {
    const v = videoRef.current;
    if (!v || !sel || !v.videoWidth) return;
    const canvas = document.createElement("canvas");
    canvas.width = v.videoWidth;
    canvas.height = v.videoHeight;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    try {
      ctx.drawImage(v, 0, 0, canvas.width, canvas.height);
      // toBlob throws SecurityError if the canvas is tainted (see crossOrigin on
      // the <video>); guard so a failure logs instead of an unhandled rejection.
      const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, "image/png"));
      if (!blob) return;
      const bytes = new Uint8Array(await blob.arrayBuffer());
      await useBoardStore.getState().extractFrame(sel, bytes);
    } catch (err) {
      void ipc.frontLog("warn", `extract frame failed: ${err}`);
    }
  };

  const fmt = (s: number) => {
    if (!Number.isFinite(s)) return "0:00";
    const m = Math.floor(s / 60);
    const ss = Math.floor(s % 60);
    return `${m}:${ss.toString().padStart(2, "0")}`;
  };

  return (
    <div ref={rootRef} className="cm-vidip" style={{ display: "none" }}>
      {show && src && (
        <>
          <div className="cm-vidip__area">
            <video
              ref={videoRef}
              className="cm-vidip__video"
              src={src}
              // REQUIRED: source served cross-origin via cameo:// (which returns
              // Access-Control-Allow-Origin: *). Without opting into CORS the
              // canvas is tainted and extractFrame's toBlob() throws — same as
              // asset-url.ts sets crossOrigin on <img> for textures.
              crossOrigin="anonymous"
              muted={muted}
              playsInline
              preload="metadata"
              onPlay={() => setPlaying(true)}
              onPause={() => setPlaying(false)}
              onTimeUpdate={(e) => setCur(e.currentTarget.currentTime)}
              onLoadedMetadata={(e) => setDur(e.currentTarget.duration)}
            />
          </div>
          <div className="cm-vidip__bar" onPointerDown={(e) => e.stopPropagation()}>
            <button className="cm-vidip__btn" title={playing ? t("vid.pause") : t("vid.play")} onClick={togglePlay}>
              {playing ? <Pause size={15} /> : <Play size={15} />}
            </button>
            <button className="cm-vidip__btn" title={t("vid.prevFrame")} onClick={() => stepFrame(-1)}>
              <ChevronLeft size={15} />
            </button>
            <button className="cm-vidip__btn" title={t("vid.nextFrame")} onClick={() => stepFrame(1)}>
              <ChevronRight size={15} />
            </button>
            <input
              className="cm-vidip__scrub"
              type="range"
              min={0}
              max={dur || 0}
              step={0.01}
              value={cur}
              onChange={onScrub}
            />
            <span className="cm-vidip__time">
              {fmt(cur)} / {fmt(dur)}
            </span>
            <button className="cm-vidip__btn" title={muted ? t("vid.unmute") : t("vid.mute")} onClick={toggleMute}>
              {muted ? <VolumeX size={15} /> : <Volume2 size={15} />}
            </button>
            <button className="cm-vidip__btn cm-vidip__btn--accent" title={t("vid.extractFrame")} onClick={() => void extractFrame()}>
              <ImagePlus size={15} />
            </button>
          </div>
        </>
      )}
    </div>
  );
}
