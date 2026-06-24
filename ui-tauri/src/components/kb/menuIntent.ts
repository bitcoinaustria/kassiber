// Pure dispatcher for native-menu and deep-link intents.
//
// Lives outside AppShell so the routing decisions are unit-testable without a
// React/Router/Tauri harness. The component wires real `navigate`, `lockApp`,
// and notification helpers; the test suite passes spies and asserts the
// resulting calls.
//
// Producer-side parsing (Rust) is covered by `lib.rs` tests — this file
// covers the consumer-side decision tree: workspace gating, the AI-route
// fallback, and the settings-section side-effect event.

export type AppRoutePath =
  | "/overview"
  | "/transactions"
  | "/activity"
  | "/reports"
  | "/exit-tax"
  | "/source-of-funds"
  | "/loans"
  | "/connections"
  | "/books"
  | "/journals"
  | "/transfers"
  | "/swaps"
  | "/quarantine"
  | "/reconcile"
  | "/logs"
  | "/settings"
  | "/assistant";

// Mirrors the Rust `DEEP_LINK_SETTINGS_SECTIONS` allowlist. Aliases
// (`sync` → backends, `assistant` → ai) round-trip from deep links and the
// native menu — drop them from this union and Rust would emit strings the
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
  | "sync"
  | "rates"
  | "ai"
  | "assistant"
  | "data"
  | "storage"
  | "desktop"
  | "terminal";

export type NativeMenuPayload =
  | { action: "lock-app" | "toggle-sensitive" }
  | { action: "ui-scale-decrease" | "ui-scale-increase" | "ui-scale-reset" }
  | { action: "add-wallet" | "sync-all-wallets" | "process-journals" }
  | { action: "open-settings"; section?: SettingsMenuSection | null }
  | { action: "navigate"; route?: AppRoutePath | null };

export const APP_ROUTE_PATHS: readonly AppRoutePath[] = [
  "/overview",
  "/transactions",
  "/activity",
  "/reports",
  "/exit-tax",
  "/source-of-funds",
  "/loans",
  "/connections",
  "/books",
  "/journals",
  "/transfers",
  "/swaps",
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

export interface MenuIntentNotification {
  title: string;
  body: string;
  tone: "info" | "warning" | "success" | "error";
}

export interface MenuIntentDeps {
  hasWorkspace: boolean;
  aiFeaturesEnabled: boolean;
  hideSensitive: boolean;
  navigate: (opts: { to: string; hash?: string }) => void;
  lockApp: () => void;
  setHideSensitive: (next: boolean) => void;
  runAddWalletConnection: () => void;
  runWalletSync: () => void;
  runJournalProcessing: () => void;
  decreaseAppScale: () => void;
  increaseAppScale: () => void;
  resetAppScale: () => void;
  addNotification: (notification: MenuIntentNotification) => void;
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
      deps.setHideSensitive(!deps.hideSensitive);
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
        deps.addNotification({
          title: "AI features are disabled",
          body: "Enable AI features in Settings to use the assistant.",
          tone: "info",
        });
        deps.navigate({ to: "/settings", hash: "ai" });
        return;
      }
      // Welcome-screen users would be bounced straight back to `/` by the
      // identity-guard effect, flashing the wrong route mid-transition.
      if (!deps.hasWorkspace) return;
      deps.navigate({ to: payload.route });
      return;
    }
  }
}
