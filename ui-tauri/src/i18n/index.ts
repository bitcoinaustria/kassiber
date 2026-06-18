/**
 * i18next bootstrap. Importing this module initializes the shared instance as
 * a side effect, so any entry point (the app via `main.tsx`, or a test via
 * `vitest.setup.ts`) only needs `import "@/i18n"` to make `useTranslation`
 * work.
 *
 * The UI store's `lang` is the single source of truth for the active language;
 * `languageBridge.ts` drives i18next from it. This module just configures the
 * instance and defaults it to English.
 */

import i18n from "i18next";
import { initReactI18next } from "react-i18next";

import { DEFAULT_LANGUAGE } from "./config";
import { defaultNS, resources } from "./resources";

if (!i18n.isInitialized) {
  void i18n.use(initReactI18next).init({
    resources,
    lng: DEFAULT_LANGUAGE,
    fallbackLng: DEFAULT_LANGUAGE,
    defaultNS,
    // Suppress i18next's promotional console banner on init.
    showSupportNotice: false,
    interpolation: {
      // React escapes interpolated values already; double-escaping mangles them.
      escapeValue: false,
    },
    // Missing keys should surface their key, never `null`, so gaps are obvious.
    returnNull: false,
    react: {
      // Resources are bundled synchronously — there is nothing to suspend on.
      useSuspense: false,
    },
  });
}

export default i18n;
