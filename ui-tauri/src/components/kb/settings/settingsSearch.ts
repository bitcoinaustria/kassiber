import * as React from "react";

import type { SettingsSectionId } from "../settingsSections";

/**
 * Setting-level search targets. App search emits one result per entry, so a
 * query for a control ("clipboard", "auto-lock") lands on its row rather than
 * only on its section. `anchor` names the row or group that carries the
 * matching `anchorId`; `labelKey` is that row's visible label (settings
 * namespace) and `keywordsKey` a comma-separated list of extra search terms.
 * Stable ids, never derived from translated text.
 */
export interface SettingsSearchEntry {
  section: SettingsSectionId;
  anchor: string;
  labelKey: string;
  keywordsKey: string;
}

const entry = <A extends string>(
  section: SettingsSectionId,
  anchor: A,
  labelKey: string,
) => ({
  section,
  anchor,
  labelKey,
  keywordsKey: `search.keywords.${anchor}`,
});

export const SETTINGS_SEARCH_ENTRIES = [
  entry("general-appearance", "theme", "appearance.theme.title"),
  entry("general-appearance", "denomination", "appearance.denomination.title"),
  entry("general-appearance", "interfaceScale", "appearance.scale.title"),
  entry("general-appearance", "language", "appearance.language.title"),
  entry("general-appearance", "assistantDock", "appearance.assistantDock.title"),

  entry("network-market", "liveRates", "marketData.autoRefreshLabel"),
  entry("network-market", "priceSource", "marketData.priceSourceLabel"),
  entry("network-market", "rebuildPricing", "marketData.rebuildHeading"),
  entry("network-market", "coarsePricingReview", "marketData.coarse.reviewToggleLabel"),
  entry("network-market", "bitcoinRails", "marketData.bitcoinRails.toggleLabel"),
  entry("network-market", "krakenHistory", "marketData.krakenHeading"),
  entry("network-market", "bundledRates", "marketData.bundledHeading"),

  entry("network-bitcoin", "bookNetwork", "bookNetwork.title"),
  entry("network-bitcoin", "bitcoinBackends", "networkLayer.bitcoin.heading"),
  entry("network-bitcoin", "analysisNetwork", "analysisNetwork.title"),
  entry("network-lightning", "lightningNodes", "networkLayer.lightning.heading"),
  entry("network-liquid", "liquidBackends", "networkLayer.liquid.heading"),

  entry("security-privacy", "offlineMode", "privacy.offlineLabel"),
  entry("security-privacy", "blurSensitive", "privacy.blurLabel"),
  entry("security-privacy", "clearClipboard", "privacy.clipboardLabel"),
  entry("security-privacy", "updateChecks", "privacy.updatesLabel"),
  entry("security-privacy", "outgoingConnections", "privacy.leavesHeading"),

  entry("security-lock", "databaseEncryption", "security.encryptionHeading"),
  entry("security-lock", "passphrase", "security.passphraseLabel"),
  entry("security-lock", "autoLock", "security.autoLockLabel"),
  entry("security-lock", "lockOnLaunch", "security.requireOnLaunchLabel"),
  entry("security-lock", "lockOnClose", "security.lockOnCloseLabel"),
  entry("security-lock", "biometricUnlock", "security.biometricHeading"),

  entry("assistant-ai", "assistantFeatures", "ai.featuresLabel"),
  entry("assistant-ai", "aiModels", "ai.modelsHeading"),
  entry("assistant-ai", "permissionMode", "ai.permissionMode.label"),
  entry("assistant-ai", "externalAgents", "ai.agentsLabel"),
  entry("assistant-ai", "chatHistory", "chatHistory.storeLabel"),

  entry("data-storage", "importData", "data.importHeading"),
  entry("data-storage", "backup", "data.backupHeading"),
  entry("data-storage", "dataRoot", "data.dataRootLabel"),
  entry("data-storage", "databasePath", "data.databaseLabel"),
  entry("data-storage", "resetWelcome", "data.resetWelcomeTitle"),
  entry("data-storage", "resetBook", "data.resetBookTitle"),
  entry("data-storage", "deleteBooks", "data.deleteBooksTitle"),

  entry("desktop-terminal", "terminalCommand", "terminal.statusLabel"),

  entry("desktop-developer", "earlyFeatures", "developer.enableLabel"),
  entry("desktop-developer", "alphaWarning", "developer.alphaBannerLabel"),
  entry("desktop-developer", "logBuffer", "developer.bufferHeading"),
] as const satisfies readonly SettingsSearchEntry[];

export type SettingsAnchorId = (typeof SETTINGS_SEARCH_ENTRIES)[number]["anchor"];

export function settingsSearchEntry(
  anchor: string,
): (typeof SETTINGS_SEARCH_ENTRIES)[number] | undefined {
  return SETTINGS_SEARCH_ENTRIES.find((candidate) => candidate.anchor === anchor);
}

/** Splits a translated keyword string ("a, b, c") into terms. */
export function settingsSearchKeywords(value: unknown): string[] {
  if (typeof value !== "string") return [];
  return value
    .split(",")
    .map((term) => term.trim())
    .filter(Boolean);
}

// ---------------------------------------------------------------------------
// Jump-to-setting: a search result asks for an anchor, the open modal scrolls
// it into view and washes it for a moment.

const HIGHLIGHT_MS = 1500;
/** How long a lazily rendered panel may take to show its row. */
const WAIT_FOR_ANCHOR_MS = 3000;

let pendingAnchor: SettingsAnchorId | null = null;
let requestedAt = 0;
const listeners = new Set<() => void>();

/** Asks the Settings modal to reveal `anchor` once its section is showing. */
export function requestSettingsAnchor(anchor: SettingsAnchorId | null) {
  pendingAnchor = anchor;
  requestedAt = Date.now();
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function prefersReducedMotion() {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/**
 * Reveals the requested anchor inside `container` once the section shows it:
 * unfolds an enclosing `<details>`, scrolls the row to the middle, and sets
 * `data-settings-highlight` for {@link HIGHLIGHT_MS}. Runs after the modal's
 * own scroll reset (a layout effect), so the reset cannot undo the jump.
 */
export function useSettingsAnchorHighlight(
  container: React.RefObject<HTMLElement | null>,
  section: SettingsSectionId,
) {
  const anchor = React.useSyncExternalStore(
    subscribe,
    () => pendingAnchor,
    () => null,
  );

  React.useEffect(() => {
    if (!anchor) return;
    const target = settingsSearchEntry(anchor);
    // A request for another section waits for that section to open; one that
    // never got there (a blocked section, a closed modal) goes stale rather
    // than jumping the next time the section is opened by hand.
    if (!target || Date.now() - requestedAt > WAIT_FOR_ANCHOR_MS) {
      requestSettingsAnchor(null);
      return;
    }
    if (target.section !== section) return;
    const root = container.current;
    if (!root) return;

    // The wash outlives this effect: clearing the request below re-runs it,
    // so only the timer takes the highlight off again.
    const reveal = () => {
      const element = root.querySelector<HTMLElement>(
        `[data-settings-anchor="${anchor}"]`,
      );
      if (!element) return false;
      const details = element.closest("details");
      if (details && !details.open) details.open = true;
      element.scrollIntoView?.({
        block: "center",
        behavior: prefersReducedMotion() ? "auto" : "smooth",
      });
      element.setAttribute("data-settings-highlight", "");
      window.setTimeout(() => {
        element.removeAttribute("data-settings-highlight");
      }, HIGHLIGHT_MS);
      requestSettingsAnchor(null);
      return true;
    };

    if (reveal()) return;
    // Daemon-backed rows render after their query settles: wait a moment for
    // them, then give up quietly (the section is open either way).
    const observer = new MutationObserver(() => {
      if (reveal()) observer.disconnect();
    });
    observer.observe(root, { childList: true, subtree: true });
    const giveUp = window.setTimeout(() => {
      observer.disconnect();
      if (pendingAnchor === anchor) requestSettingsAnchor(null);
    }, WAIT_FOR_ANCHOR_MS);
    return () => {
      observer.disconnect();
      window.clearTimeout(giveUp);
    };
  }, [anchor, container, section]);
}
