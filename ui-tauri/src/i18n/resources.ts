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

import deAssistant from "@shared/i18n/locales/de/assistant.json";
import deChainAnalysis from "@shared/i18n/locales/de/chainAnalysis.json";
import enChainAnalysis from "@shared/i18n/locales/en/chainAnalysis.json";
import deChrome from "@shared/i18n/locales/de/chrome.json";
import deCommon from "@shared/i18n/locales/de/common.json";
import deConnections from "@shared/i18n/locales/de/connections.json";
import deCustodyGaps from "@shared/i18n/locales/de/custodyGaps.json";
import deJournals from "@shared/i18n/locales/de/journals.json";
import deNav from "@shared/i18n/locales/de/nav.json";
import deOnboarding from "@shared/i18n/locales/de/onboarding.json";
import deOverview from "@shared/i18n/locales/de/overview.json";
import dePrivacyMirror from "@shared/i18n/locales/de/privacyMirror.json";
import deReview from "@shared/i18n/locales/de/review.json";
import deSearch from "@shared/i18n/locales/de/search.json";
import deSettings from "@shared/i18n/locales/de/settings.json";
import deSourceFunds from "@shared/i18n/locales/de/sourceFunds.json";
import deTransactions from "@shared/i18n/locales/de/transactions.json";
import enAssistant from "@shared/i18n/locales/en/assistant.json";
import enChrome from "@shared/i18n/locales/en/chrome.json";
import enCommon from "@shared/i18n/locales/en/common.json";
import enConnections from "@shared/i18n/locales/en/connections.json";
import enCustodyGaps from "@shared/i18n/locales/en/custodyGaps.json";
import enJournals from "@shared/i18n/locales/en/journals.json";
import enNav from "@shared/i18n/locales/en/nav.json";
import enOnboarding from "@shared/i18n/locales/en/onboarding.json";
import enOverview from "@shared/i18n/locales/en/overview.json";
import enPrivacyMirror from "@shared/i18n/locales/en/privacyMirror.json";
import enReview from "@shared/i18n/locales/en/review.json";
import enSearch from "@shared/i18n/locales/en/search.json";
import enSettings from "@shared/i18n/locales/en/settings.json";
import enSourceFunds from "@shared/i18n/locales/en/sourceFunds.json";
import enTransactions from "@shared/i18n/locales/en/transactions.json";

/** Namespace consulted when a `t("key")` call carries no `ns:` prefix. */
export const defaultNS = "common";

export const resources = {
  en: {
    common: enCommon,
    nav: enNav,
    chrome: enChrome,
    settings: enSettings,
    overview: enOverview,
    privacyMirror: enPrivacyMirror,
    chainAnalysis: enChainAnalysis,
    transactions: enTransactions,
    connections: enConnections,
    custodyGaps: enCustodyGaps,
    journals: enJournals,
    onboarding: enOnboarding,
    assistant: enAssistant,
    sourceFunds: enSourceFunds,
    review: enReview,
    search: enSearch,
  },
  de: {
    common: deCommon,
    nav: deNav,
    chrome: deChrome,
    settings: deSettings,
    overview: deOverview,
    privacyMirror: dePrivacyMirror,
    chainAnalysis: deChainAnalysis,
    transactions: deTransactions,
    connections: deConnections,
    custodyGaps: deCustodyGaps,
    journals: deJournals,
    onboarding: deOnboarding,
    assistant: deAssistant,
    sourceFunds: deSourceFunds,
    review: deReview,
    search: deSearch,
  },
} as const;

/** Shape of one language's bundles — drives the type-safe `t()` keys. */
export type AppResources = (typeof resources)["en"];
export type Namespace = keyof AppResources;

export const NAMESPACES = Object.keys(resources.en) as Namespace[];
