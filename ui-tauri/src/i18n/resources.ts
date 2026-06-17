/**
 * Static resource bundles. Translations are bundled into the app so the active
 * language switches synchronously with no flash of untranslated content.
 *
 * Scaling path: when the language or string count grows enough that bundling
 * everything hurts the initial payload, swap this for `i18next-http-backend`
 * (or dynamic `import()` per namespace) and lazy-load on demand. The rest of
 * the app talks to `t(...)` and does not care how resources arrive — only this
 * module and `index.ts` would change. See docs/reference/i18n.md.
 *
 * Namespaces are per-surface (one screen/feature family each), plus the shared
 * `common` (reusable UI vocabulary) and `nav` (navigation labels). Keep `en`
 * and `de` in lockstep: every key in one must exist in the other.
 * `i18n.test.ts` enforces that.
 */

import deAssistant from "./locales/de/assistant.json";
import deChrome from "./locales/de/chrome.json";
import deCommon from "./locales/de/common.json";
import deConnections from "./locales/de/connections.json";
import deJournals from "./locales/de/journals.json";
import deNav from "./locales/de/nav.json";
import deOnboarding from "./locales/de/onboarding.json";
import deOverview from "./locales/de/overview.json";
import deReview from "./locales/de/review.json";
import deSettings from "./locales/de/settings.json";
import deSourceFunds from "./locales/de/sourceFunds.json";
import deTransactions from "./locales/de/transactions.json";
import enAssistant from "./locales/en/assistant.json";
import enChrome from "./locales/en/chrome.json";
import enCommon from "./locales/en/common.json";
import enConnections from "./locales/en/connections.json";
import enJournals from "./locales/en/journals.json";
import enNav from "./locales/en/nav.json";
import enOnboarding from "./locales/en/onboarding.json";
import enOverview from "./locales/en/overview.json";
import enReview from "./locales/en/review.json";
import enSettings from "./locales/en/settings.json";
import enSourceFunds from "./locales/en/sourceFunds.json";
import enTransactions from "./locales/en/transactions.json";

/** Namespace consulted when a `t("key")` call carries no `ns:` prefix. */
export const defaultNS = "common";

export const resources = {
  en: {
    common: enCommon,
    nav: enNav,
    chrome: enChrome,
    settings: enSettings,
    overview: enOverview,
    transactions: enTransactions,
    connections: enConnections,
    journals: enJournals,
    onboarding: enOnboarding,
    assistant: enAssistant,
    sourceFunds: enSourceFunds,
    review: enReview,
  },
  de: {
    common: deCommon,
    nav: deNav,
    chrome: deChrome,
    settings: deSettings,
    overview: deOverview,
    transactions: deTransactions,
    connections: deConnections,
    journals: deJournals,
    onboarding: deOnboarding,
    assistant: deAssistant,
    sourceFunds: deSourceFunds,
    review: deReview,
  },
} as const;

/** Shape of one language's bundles — drives the type-safe `t()` keys. */
export type AppResources = (typeof resources)["en"];
export type Namespace = keyof AppResources;

export const NAMESPACES = Object.keys(resources.en) as Namespace[];
