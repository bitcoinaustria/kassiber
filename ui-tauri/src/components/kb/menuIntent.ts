// Pure dispatcher for native-menu and deep-link intents.
//
// Lives outside AppShell so the routing decisions are unit-testable without a
// React/Router/Tauri harness. The component wires real `navigate`, `lockApp`,
// and settings-anchor helpers; the test suite passes spies and asserts the
// resulting calls.
//
// Producer-side parsing (Rust) is covered by `lib.rs` tests — this file
// covers the consumer-side decision tree: workspace gating, the AI-route
// fallback, and the settings-section side-effect event.

import { isDevOnlyRoute, isDevOnlySettingsSlug } from "./devMode";
import type { SettingsAnchorId } from "./settings/settingsSearch";

export type AppRoutePath =
  | "/overview"
  | "/transactions"
  | "/activity"
  | "/reports"
  | "/privacy-mirror"
  | "/chain-analysis"
  | "/exit-tax"
  | "/source-of-funds"
  | "/egress"
  | "/connections"
  | "/books"
  | "/journals"
  | "/transfers"
  | "/swaps"
  | "/custody-gaps"
  | "/review"
  | "/quarantine"
  | "/reconcile"
  | "/logs"
  | "/settings"
  | "/assistant";

// Mirrors the Rust `DEEP_LINK_SETTINGS_SECTIONS` allowlist. Aliases
// (`assistant` → ai, `backends` → bitcoin, …) round-trip from deep
// links and the native menu — drop them from this union and Rust would emit strings the
// type system says are impossible.
export type SettingsMenuSection =
  | "appearance"
  | "privacy"
  | "developer"
  | "logs"
  | "display"
  | "explorer"
  | "explorers"
  | "bitcoin"
  | "lightning"
  | "liquid"
  | "market"
  | "security"
  | "lock"
  | "backends"
  | "rates"
  | "ai"
  | "assistant"
  | "data"
  | "storage"
  | "desktop"
  | "terminal";

export type NativeMenuPayload =
  | { action: "lock-app" | "toggle-sensitive" | "check-for-updates" }
  | { action: "ui-scale-decrease" | "ui-scale-increase" | "ui-scale-reset" }
  | { action: "add-wallet" | "sync-all-wallets" | "process-journals" }
  | { action: "open-settings"; section?: SettingsMenuSection | null }
  | { action: "navigate"; route?: AppRoutePath | null };

export const APP_ROUTE_PATHS: readonly AppRoutePath[] = [
  "/overview",
  "/transactions",
  "/activity",
  "/reports",
  "/privacy-mirror",
  "/chain-analysis",
  "/exit-tax",
  "/source-of-funds",
  "/egress",
  "/connections",
  "/books",
  "/journals",
  "/transfers",
  "/swaps",
  "/custody-gaps",
  "/review",
  "/quarantine",
  "/reconcile",
  "/logs",
  "/settings",
  "/assistant",
];

export function isAppRoutePath(value: unknown): value is AppRoutePath {
  return (
    typeof value === "string" &&
    APP_ROUTE_PATHS.includes(value as AppRoutePath)
  );
}

export interface MenuIntentDeps {
  hasWorkspace: boolean;
  aiFeaturesEnabled: boolean;
  developerToolsEnabled: boolean;
  navigate: (opts: { to: string; hash?: string }) => void;
  lockApp: () => void;
  /** Normal → Hide amounts → Hide all → Normal. */
  cyclePrivacyMode: () => void;
  runAddWalletConnection: () => void;
  runWalletSync: () => void;
  runJournalProcessing: () => void;
  decreaseAppScale: () => void;
  increaseAppScale: () => void;
  resetAppScale: () => void;
  runAppUpdateCheck: () => void;
  /**
   * Reveal one settings row once its section shows: a gated destination
   * explains itself by highlighting the switch that opens it, instead of a
   * notice.
   */
  revealSettingAnchor: (anchor: SettingsAnchorId) => void;
  emitSettingsSection: (section: string | null) => void;
}

// `global` actions are workspace-independent (route navigation, settings
// panels, sensitive toggle) and must be reachable even on the Welcome screen
// before AppShell mounts. `workspace` actions require AppShell-scoped
// runners (lockApp + workflow mutations) and only fire there. Two listeners
// (one at the root layout, one inside AppShell) split the surface so neither
// double-handles an event.
export type MenuIntentScope = "global" | "workspace" | "all";

const GLOBAL_ACTIONS = new Set([
  "navigate",
  "open-settings",
  "toggle-sensitive",
  "ui-scale-decrease",
  "ui-scale-increase",
  "ui-scale-reset",
  "check-for-updates",
] as const);

function actionScope(action: NativeMenuPayload["action"]): "global" | "workspace" {
  return GLOBAL_ACTIONS.has(action as (typeof GLOBAL_ACTIONS extends Set<infer T> ? T : never))
    ? "global"
    : "workspace";
}

export function dispatchMenuIntent(
  payload: NativeMenuPayload,
  deps: MenuIntentDeps,
  scope: MenuIntentScope = "all",
): void {
  // Scope filter: callers pass `"global"` from the root layout (always
  // mounted) and `"workspace"` from inside AppShell (mounted only when
  // identity exists). `"all"` is for tests and any single-listener setups.
  // Strictly disjoint dispatch avoids double-handling toggle-sensitive etc.
  if (scope !== "all" && actionScope(payload.action) !== scope) {
    return;
  }
  // Workspace gating note: `lock-app` and `navigate` are gated here against
  // `deps.hasWorkspace`. The workflow actions (`add-wallet`,
  // `sync-all-wallets`, `process-journals`) are gated *inside* their runners
  // — see AppShell, which calls `ensureWorkspaceForMenuAction()` before
  // mutating or opening workspace-scoped dialogs. If the gating rule ever
  // changes (e.g. allow some workflow during onboarding), update both sites.
  switch (payload.action) {
    case "lock-app":
      // The native menu greys out Lock when there's no workspace, but the
      // deep-link surface (`kassiber://lock`) bypasses that — silently drop
      // so the two surfaces stay symmetric.
      if (!deps.hasWorkspace) return;
      deps.lockApp();
      return;

    case "toggle-sensitive":
      deps.cyclePrivacyMode();
      return;

    case "ui-scale-decrease":
      deps.decreaseAppScale();
      return;

    case "ui-scale-increase":
      deps.increaseAppScale();
      return;

    case "ui-scale-reset":
      deps.resetAppScale();
      return;

    case "check-for-updates":
      deps.runAppUpdateCheck();
      return;

    case "add-wallet":
      deps.runAddWalletConnection();
      return;

    case "sync-all-wallets":
      deps.runWalletSync();
      return;

    case "process-journals":
      deps.runJournalProcessing();
      return;

    case "open-settings":
      // `lightning` resolves to a section the router hides while
      // early-stage features are off; say so instead of bouncing to Overview.
      if (
        !deps.developerToolsEnabled &&
        isDevOnlySettingsSlug(payload.section)
      ) {
        deps.revealSettingAnchor("earlyFeatures");
        deps.navigate({ to: "/settings", hash: "developer" });
        deps.emitSettingsSection("developer");
        return;
      }
      // (The section event is re-fired here, unlike the navigate case above,
      // because Settings may already be mounted on another section.)
      deps.navigate({
        to: "/settings",
        hash: payload.section ?? undefined,
      });
      // Re-fire the section so the SettingsScreen panel re-opens even when
      // the URL hash didn't change (user clicked the same menu item twice).
      deps.emitSettingsSection(payload.section ?? null);
      return;

    case "navigate": {
      if (!isAppRoutePath(payload.route)) return;
      if (payload.route === "/assistant" && !deps.aiFeaturesEnabled) {
        deps.revealSettingAnchor("assistantFeatures");
        deps.navigate({ to: "/settings", hash: "ai" });
        return;
      }
      // Welcome-screen users would be bounced straight back to `/` by the
      // identity-guard effect, flashing the wrong route mid-transition. This
      // stays above the early-stage branch below: with no book open there is
      // nowhere to land, and nothing should wait for after the unlock as a
      // phantom reaction to something the user did before it.
      if (!deps.hasWorkspace) return;
      // Same shape as the AI branch: the router would send these to Overview,
      // which reads as a broken menu item. Point at the switch instead.
      if (!deps.developerToolsEnabled && isDevOnlyRoute(payload.route)) {
        deps.revealSettingAnchor("earlyFeatures");
        deps.navigate({ to: "/settings", hash: "developer" });
        return;
      }
      deps.navigate({ to: payload.route });
      return;
    }
  }
}
