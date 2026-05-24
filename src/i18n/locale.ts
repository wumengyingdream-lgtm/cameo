import { create } from "zustand";
import { messages, type MsgKey } from "./messages";

export type LocaleChoice = "system" | "en" | "zh";
type Lang = "en" | "zh";

const STORE_KEY = "cameo.locale";

function systemLang(): Lang {
  return typeof navigator !== "undefined" && navigator.language?.toLowerCase().startsWith("zh") ? "zh" : "en";
}
function readChoice(): LocaleChoice {
  const v = (typeof localStorage !== "undefined" && localStorage.getItem(STORE_KEY)) || "system";
  return v === "en" || v === "zh" ? v : "system";
}
function resolve(choice: LocaleChoice): Lang {
  return choice === "system" ? systemLang() : choice;
}

interface LocaleState {
  /** User's explicit choice (persisted); "system" follows navigator.language. */
  choice: LocaleChoice;
  /** Resolved language actually rendered. */
  lang: Lang;
  setChoice: (c: LocaleChoice) => void;
}

export const useLocaleStore = create<LocaleState>((set) => {
  const choice = readChoice();
  return {
    choice,
    lang: resolve(choice),
    setChoice: (c) => {
      try {
        localStorage.setItem(STORE_KEY, c);
      } catch {
        /* private mode / unavailable — fall back to in-memory */
      }
      set({ choice: c, lang: resolve(c) });
    },
  };
});

/** A translate function bound to the current language; components re-render when
 *  the language changes. Supports `{name}` interpolation. */
export function useT() {
  const lang = useLocaleStore((s) => s.lang);
  return (key: MsgKey, params?: Record<string, string | number>): string => {
    let s: string = messages[lang][key] ?? messages.en[key] ?? key;
    if (params) {
      for (const k in params) s = s.split(`{${k}}`).join(String(params[k]));
    }
    return s;
  };
}
