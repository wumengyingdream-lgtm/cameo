import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { useUiStore } from "../store/ui";

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

/** Two-image comparison overlay: a draggable before/after slider (lineage) or
 *  a side-by-side view (any two images). */
export function CompareModal() {
  const compare = useUiStore((s) => s.compare);
  const [pos, setPos] = useState(50);
  const dragging = useRef(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") useUiStore.getState().setCompare(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (!compare) return null;
  const close = () => useUiStore.getState().setCompare(null);

  if (compare.mode === "side") {
    return (
      <div className="cm-compare" onClick={close}>
        <div className="cm-compare__side" onClick={(e) => e.stopPropagation()}>
          <figure className="cm-compare__fig">
            <img src={compare.beforeUrl} alt="" />
            <figcaption>{compare.beforeLabel}</figcaption>
          </figure>
          <figure className="cm-compare__fig">
            <img src={compare.afterUrl} alt="" />
            <figcaption>{compare.afterLabel}</figcaption>
          </figure>
        </div>
        <button className="cm-compare__close" onClick={close}>
          <X size={16} />
        </button>
      </div>
    );
  }

  const update = (clientX: number) => {
    const r = ref.current?.getBoundingClientRect();
    if (r) setPos(clamp(((clientX - r.left) / r.width) * 100, 0, 100));
  };

  return (
    <div className="cm-compare" onClick={close}>
      <div
        ref={ref}
        className="cm-compare__slider"
        onClick={(e) => e.stopPropagation()}
        onPointerDown={(e) => {
          dragging.current = true;
          update(e.clientX);
        }}
        onPointerMove={(e) => dragging.current && update(e.clientX)}
        onPointerUp={() => (dragging.current = false)}
        onPointerLeave={() => (dragging.current = false)}
      >
        <img className="cm-compare__img" src={compare.beforeUrl} alt="" draggable={false} />
        <img
          className="cm-compare__img cm-compare__img--top"
          src={compare.afterUrl}
          alt=""
          draggable={false}
          style={{ clipPath: `inset(0 0 0 ${pos}%)` }}
        />
        <div className="cm-compare__divider" style={{ left: `${pos}%` }} />
        <span className="cm-compare__lbl cm-compare__lbl--l">{compare.beforeLabel}</span>
        <span className="cm-compare__lbl cm-compare__lbl--r">{compare.afterLabel}</span>
      </div>
      <button className="cm-compare__close" onClick={close}>
        <X size={16} />
      </button>
    </div>
  );
}
