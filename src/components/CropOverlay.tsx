import { useEffect, useRef, useState, type RefObject } from "react";
import { useUiStore } from "../store/ui";
import { useBoardStore } from "../store/board";
import { bakeCrop } from "../lib/crop";
import { useT } from "../i18n/locale";

interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}
type Corner = "tl" | "tr" | "br" | "bl";
const MIN = 0.05;
const clamp01 = (v: number) => Math.max(0, Math.min(1, v));
const DEFAULT: Rect = { x: 0.05, y: 0.05, w: 0.9, h: 0.9 };

const PRESETS: { label: string; ratio: number | null }[] = [
  { label: "", ratio: null }, // label resolved via t("crop.free")
  { label: "1:1", ratio: 1 },
  { label: "4:5", ratio: 4 / 5 },
  { label: "3:2", ratio: 3 / 2 },
  { label: "16:9", ratio: 16 / 9 },
  { label: "9:16", ratio: 9 / 16 },
];

/** In-place crop: the crop frame is drawn ON the selected image (the scene
 *  positions `rootRef` to the image's screen rect each frame, so it follows the
 *  image), with a panel floating to its right. 完成 bakes a cropped Asset. */
export function CropOverlay({ rootRef }: { rootRef: RefObject<HTMLDivElement | null> }) {
  const cropping = useUiStore((s) => s.cropping);
  const placements = useBoardStore((s) => s.placements);
  const assets = useBoardStore((s) => s.assets);
  const boardId = useBoardStore((s) => s.boardId);
  const [crop, setCrop] = useState<Rect>(DEFAULT);
  const drag = useRef<{ mode: "move" | Corner; sx: number; sy: number; start: Rect } | null>(null);
  const t = useT();

  useEffect(() => {
    setCrop(DEFAULT);
  }, [cropping]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") useUiStore.getState().setCropping(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // Drag listeners live for the component lifetime (no-op unless dragging).
  useEffect(() => {
    const onMove = (e: PointerEvent) => {
      const d = drag.current;
      const root = rootRef.current;
      if (!d || !root) return;
      const r = root.getBoundingClientRect();
      const dx = (e.clientX - d.sx) / r.width;
      const dy = (e.clientY - d.sy) / r.height;
      const s = d.start;
      setCrop(() => {
        if (d.mode === "move") {
          const x = Math.max(0, clamp01(Math.min(s.x + dx, 1 - s.w)));
          const y = Math.max(0, clamp01(Math.min(s.y + dy, 1 - s.h)));
          return { x, y, w: s.w, h: s.h };
        }
        let { x, y, w, h } = s;
        const right = s.x + s.w;
        const bottom = s.y + s.h;
        if (d.mode === "tl" || d.mode === "bl") {
          x = clamp01(Math.min(s.x + dx, right - MIN));
          w = right - x;
        } else {
          w = Math.max(MIN, clamp01(Math.min(s.w + dx, 1 - s.x)));
        }
        if (d.mode === "tl" || d.mode === "tr") {
          y = clamp01(Math.min(s.y + dy, bottom - MIN));
          h = bottom - y;
        } else {
          h = Math.max(MIN, clamp01(Math.min(s.h + dy, 1 - s.y)));
        }
        return { x, y, w, h };
      });
    };
    const onUp = () => {
      drag.current = null;
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
  }, [rootRef]);

  const p = cropping ? placements.get(cropping) : null;
  const a = p && assets.get(p.assetId);
  const show = !!cropping && !!a && !!boardId;

  const startDrag = (mode: "move" | Corner) => (e: React.PointerEvent) => {
    e.preventDefault();
    e.stopPropagation();
    drag.current = { mode, sx: e.clientX, sy: e.clientY, start: crop };
  };
  const applyRatio = (ratio: number | null) => {
    if (ratio == null || !a) return;
    const rn = ratio * (a.height / a.width);
    let w = 1;
    let h = 1;
    if (rn >= 1) h = 1 / rn;
    else w = rn;
    setCrop({ x: (1 - w) / 2, y: (1 - h) / 2, w, h });
  };
  const close = () => useUiStore.getState().setCropping(null);
  const done = async () => {
    if (!a || !boardId || !cropping) return;
    await useBoardStore.getState().replacePlacementImage(cropping, await bakeCrop(boardId, a, crop));
    close();
  };

  const pct = (v: number) => `${v * 100}%`;
  return (
    <div ref={rootRef} className="cm-cropip" style={{ display: "none" }}>
      {show && a && (
        <>
          <div className="cm-cropip__area">
            <div
              className="cm-cropip__frame"
              style={{ left: pct(crop.x), top: pct(crop.y), width: pct(crop.w), height: pct(crop.h) }}
              onPointerDown={startDrag("move")}
            >
              {(["tl", "tr", "br", "bl"] as Corner[]).map((c) => (
                <span key={c} className={`cm-cropip__h cm-cropip__h--${c}`} onPointerDown={startDrag(c)} />
              ))}
            </div>
          </div>
          <div className="cm-cropip__panel" onPointerDown={(e) => e.stopPropagation()}>
            <div className="cm-cropip__title">{t("crop.title")}</div>
            <div className="cm-cropip__dim">
              W {Math.round(crop.w * a.width)} × H {Math.round(crop.h * a.height)}
            </div>
            <div className="cm-cropip__presets">
              {PRESETS.map((pre) => (
                <button key={pre.ratio ?? "free"} className="cm-cropip__preset" onClick={() => applyRatio(pre.ratio)}>
                  {pre.ratio === null ? t("crop.free") : pre.label}
                </button>
              ))}
            </div>
            <div className="cm-cropip__actions">
              <button className="cm-btn" onClick={close}>
                {t("crop.cancel")}
              </button>
              <button className="cm-btn cm-btn--primary" onClick={() => void done()}>
                {t("crop.done")}
              </button>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
