/**
 * One-way bridge: the UI store's `lang` is the single source of truth, and
 * i18next plus the `<html lang>` attribute follow it. This avoids the classic
 * two-sources-of-truth bug where a browser language detector and a persisted
 * preference disagree.
 *
 * Call `installLanguageBridge()` once during app bootstrap (`main.tsx`).
 */

import { useUiStore } from "@/store/ui";

import { DEFAULT_LANGUAGE, isSupportedLanguage } from "./config";
import i18n from "./index";

function applyLanguage(lang: string): void {
  const next = isSupportedLanguage(lang) ? lang : DEFAULT_LANGUAGE;
  if (i18n.language !== next) {
    void i18n.changeLanguage(next);
  }
  if (typeof document !== "undefined") {
    document.documentElement.lang = next;
  }
}

/**
 * Apply the persisted language immediately, then keep i18next in sync with
 * every later `setLang`. Returns the store unsubscribe handle.
 */
export function installLanguageBridge(): () => void {
  let previous = useUiStore.getState().lang;
  applyLanguage(previous);
  return useUiStore.subscribe((state) => {
    if (state.lang !== previous) {
      previous = state.lang;
      applyLanguage(state.lang);
    }
  });
}
