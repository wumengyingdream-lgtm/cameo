import { useEffect, useRef, useState } from "react";
import { Check, ChevronRight, Zap } from "lucide-react";
import { useT } from "../i18n/locale";
import type { MsgKey } from "../i18n/messages";
import { useBoardStore } from "../store/board";
import {
  useGenStore,
  effortsFor,
  tiersFor,
  type Effort,
} from "../store/genSettings";
import type { ModelInfo } from "../types";

/**
 * The composer's "model / 智能 / 速度" picker — mirrors the official Codex app's
 * input-box menu. A trigger pill shows `{model} {effort}`; clicking opens a
 * popover (opens upward, the composer sits at the bottom) with the effort list
 * inline and Model / Speed as left-flyout submenus, all driven by `model/list`.
 */

const EFFORT_KEY: Record<Effort, MsgKey> = {
  low: "composer.gen.effort.low",
  medium: "composer.gen.effort.medium",
  high: "composer.gen.effort.high",
  xhigh: "composer.gen.effort.xhigh",
};

/** "GPT-5.5" / "gpt-5.5" → "5.5" for the compact trigger label. */
function shortModel(label: string): string {
  return label.replace(/^gpt-/i, "");
}

function modelLabel(id: string, models: ModelInfo[]): string {
  return models.find((m) => m.id === id)?.displayName ?? id;
}

type Sub = "model" | "speed" | null;

export function GenSettingsMenu() {
  const t = useT();
  const boardId = useBoardStore((s) => s.boardId);
  const model = useGenStore((s) => s.model);
  const effort = useGenStore((s) => s.effort);
  const serviceTier = useGenStore((s) => s.serviceTier);
  const models = useGenStore((s) => s.models);

  const [open, setOpen] = useState(false);
  const [sub, setSub] = useState<Sub>(null);
  const rootRef = useRef<HTMLDivElement>(null);

  // Dismiss on outside pointerdown / Escape (mirrors CanvasContextMenu).
  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) {
        setOpen(false);
        setSub(null);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
        setSub(null);
      }
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const current = models.find((m) => m.id === model);
  const efforts = effortsFor(current);
  const tiers = tiersFor(current);
  const effortLabel = (e: string) =>
    (EXPOSED(e) ? t(EFFORT_KEY[e as Effort]) : e);

  const pick = (fn: () => void) => {
    fn();
    setOpen(false);
    setSub(null);
  };

  const triggerLabel = `${shortModel(modelLabel(model, models))} ${effortLabel(effort)}`;
  const tierLabel = serviceTier
    ? tiers.find((tr) => tr.id === serviceTier)?.name ?? serviceTier
    : t("composer.gen.speed.standard");

  return (
    <div className="cm-gen" ref={rootRef}>
      <button
        type="button"
        className="cm-gen__trigger"
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={!boardId}
        title={t("composer.gen.menu")}
        onClick={() => {
          setOpen((v) => !v);
          setSub(null);
        }}
      >
        <span className="cm-gen__triggerlabel">{triggerLabel}</span>
        <svg width="9" height="6" viewBox="0 0 9 6" aria-hidden className="cm-gen__caret">
          <path d="M1 1l3.5 3.5L8 1" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>

      {open && (
        <div className="cm-gen__menu" role="menu">
          <div className="cm-gen__header">{t("composer.gen.intelligence")}</div>
          {efforts.map((e) => (
            <button
              key={e}
              type="button"
              className="cm-gen__item"
              role="menuitemradio"
              aria-checked={effort === e}
              onMouseEnter={() => setSub(null)}
              onClick={() => pick(() => useGenStore.getState().setEffort(e))}
            >
              <span className="cm-gen__label">{effortLabel(e)}</span>
              {effort === e && <Check size={15} className="cm-gen__check" />}
            </button>
          ))}

          <div className="cm-gen__sep" />

          {/* Model flyout */}
          <div className="cm-gen__row" onMouseEnter={() => setSub("model")}>
            <button type="button" className="cm-gen__item" role="menuitem">
              <span className="cm-gen__label">{modelLabel(model, models)}</span>
              <ChevronRight size={15} className="cm-gen__chev" />
            </button>
            {sub === "model" && models.length > 0 && (
              <div className="cm-gen__flyout" role="menu">
                <div className="cm-gen__header">{t("composer.gen.model")}</div>
                {models.map((m) => (
                  <button
                    key={m.id}
                    type="button"
                    className="cm-gen__item"
                    role="menuitemradio"
                    aria-checked={model === m.id}
                    onClick={() => pick(() => useGenStore.getState().setModel(m.id))}
                  >
                    <span className="cm-gen__label">{m.displayName}</span>
                    {model === m.id && <Check size={15} className="cm-gen__check" />}
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Speed flyout */}
          <div className="cm-gen__row" onMouseEnter={() => setSub("speed")}>
            <button type="button" className="cm-gen__item" role="menuitem">
              <span className="cm-gen__label">{t("composer.gen.speed")}</span>
              <span className="cm-gen__value">{tierLabel}</span>
              <ChevronRight size={15} className="cm-gen__chev" />
            </button>
            {sub === "speed" && (
              <div className="cm-gen__flyout" role="menu">
                <div className="cm-gen__header">{t("composer.gen.speed")}</div>
                <button
                  type="button"
                  className="cm-gen__item cm-gen__item--rich"
                  role="menuitemradio"
                  aria-checked={serviceTier === null}
                  onClick={() => pick(() => useGenStore.getState().setServiceTier(null))}
                >
                  <span className="cm-gen__rich">
                    <span className="cm-gen__label">{t("composer.gen.speed.standard")}</span>
                    <span className="cm-gen__desc">{t("composer.gen.speed.standardDesc")}</span>
                  </span>
                  {serviceTier === null && <Check size={15} className="cm-gen__check" />}
                </button>
                {tiers.map((tr) => (
                  <button
                    key={tr.id}
                    type="button"
                    className="cm-gen__item cm-gen__item--rich"
                    role="menuitemradio"
                    aria-checked={serviceTier === tr.id}
                    onClick={() => pick(() => useGenStore.getState().setServiceTier(tr.id))}
                  >
                    <span className="cm-gen__rich">
                      <span className="cm-gen__label">
                        <Zap size={13} className="cm-gen__zap" /> {tr.name}
                      </span>
                      {tr.description && <span className="cm-gen__desc">{tr.description}</span>}
                    </span>
                    {serviceTier === tr.id && <Check size={15} className="cm-gen__check" />}
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

/** True for the four exposed effort levels (others fall back to the raw id). */
function EXPOSED(e: string): e is Effort {
  return e === "low" || e === "medium" || e === "high" || e === "xhigh";
}
