import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from "react";
import { Play, Pause, Volume2, VolumeX, ImagePlus } from "lucide-react";
import { useBoardStore } from "../store/board";
import { useUiStore } from "../store/ui";
import { useVideoPlaybackStore } from "../store/videoPlayback";
import { cameoUrl, ipc } from "../lib/ipc";
import { isVideoAsset } from "../lib/media";
import { useT } from "../i18n/locale";

/**
 * The on-canvas video player. Like CropOverlay/SelectionBar, the PixiJS scene
 * positions `rootRef` every frame (image rect in screen space) so the player
 * tracks pan/zoom/drag — React only owns the content. Shown for a single video
 * selection that isn't being cropped.
 *
 * Design (PRD §6.5, decision E2; controls per §17/F4):
 *  - The `<video>` is `pointer-events: none`, so canvas drag/select/delete keep
 *    working *through* it (the poster sprite underneath is the hit target). The
 *    control bar alone captures pointer.
 *  - Only the focused video gets a live `<video>`; every other video on the
 *    board stays a static poster sprite — N videos cost ~N images, not N decode
 *    pipelines (avoids the SP-3 perf cliff).
 *  - Scrub seeks the `cameo://` source, which serves HTTP Range (206) — without
 *    that the element refuses to seek. crossOrigin lets extractFrame read pixels.
 *  - The bar is play/pause + a drag scrubber (live frame preview, stays where
 *    released) + time + mute + extract-frame. It registers play/pause + scrub
 *    state into the video-playback store so spacebar and the "reference" button
 *    can drive / read the focused video.
 */
export function VideoOverlay({ rootRef }: { rootRef: RefObject<HTMLDivElement | null> }) {
  const selection = useBoardStore((s) => s.selection);
  const placements = useBoardStore((s) => s.placements);
  const assets = useBoardStore((s) => s.assets);
  const boardId = useBoardStore((s) => s.boardId);
  const cropping = useUiStore((s) => s.cropping);
  const t = useT();

  const videoRef = useRef<HTMLVideoElement>(null);
  const scrubRef = useRef<HTMLDivElement>(null);
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

  // togglePlay only reads the stable videoRef, so it's safe to register once.
  const togglePlay = useCallback(() => {
    const v = videoRef.current;
    if (!v) return;
    if (v.paused) void v.play();
    else v.pause();
  }, []);

  // Reset transient playback state when the selected video changes.
  useEffect(() => {
    setPlaying(false);
    setCur(0);
    setDur(0);
  }, [src]);

  // Register this video's controls so the scene (spacebar) and SelectionBar
  // (reference) can reach it; unregister when it's no longer the live player.
  useEffect(() => {
    if (show && sel) {
      useVideoPlaybackStore.getState().register(sel, togglePlay);
      const id = sel;
      return () => useVideoPlaybackStore.getState().unregister(id);
    }
  }, [show, sel, togglePlay]);

  const seekToClientX = (clientX: number) => {
    const v = videoRef.current;
    const el = scrubRef.current;
    if (!v || !el || !dur) return;
    const rect = el.getBoundingClientRect();
    const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
    const time = ratio * dur;
    // Precise assignment (not fastSeek) so WKWebView actually renders the frame
    // under the cursor — the live preview the user expects while dragging.
    v.currentTime = time;
    setCur(time);
    useVideoPlaybackStore.getState().setTime(time, true);
  };

  const onScrubDown = (e: React.PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    videoRef.current?.pause();
    e.currentTarget.setPointerCapture(e.pointerId);
    seekToClientX(e.clientX);
  };

  const onScrubMove = (e: React.PointerEvent) => {
    if (e.buttons === 0) return; // only while dragging
    seekToClientX(e.clientX);
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

  const progress = dur > 0 ? Math.max(0, Math.min(1, cur / dur)) * 100 : 0;

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
              onPlay={() => {
                setPlaying(true);
                useVideoPlaybackStore.getState().setPlaying(true);
              }}
              onPause={() => {
                setPlaying(false);
                useVideoPlaybackStore.getState().setPlaying(false);
              }}
              onTimeUpdate={(e) => {
                const time = e.currentTarget.currentTime;
                setCur(time);
                useVideoPlaybackStore.getState().setTime(time);
              }}
              onLoadedMetadata={(e) => {
                const d = e.currentTarget.duration;
                setDur(d);
                useVideoPlaybackStore.getState().setDuration(d);
              }}
            />
          </div>
          <div className="cm-vidip__bar" onPointerDown={(e) => e.stopPropagation()}>
            <button className="cm-vidip__btn" title={playing ? t("vid.pause") : t("vid.play")} onClick={togglePlay}>
              {playing ? <Pause size={15} /> : <Play size={15} />}
            </button>
            <div
              ref={scrubRef}
              className="cm-vidip__scrub"
              onPointerDown={onScrubDown}
              onPointerMove={onScrubMove}
            >
              <div className="cm-vidip__scrub-fill" style={{ width: `${progress}%` }} />
              <div className="cm-vidip__scrub-knob" style={{ left: `${progress}%` }} />
            </div>
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
