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
  | "/reports"
  | "/source-of-funds"
  | "/connections"
  | "/books"
  | "/journals"
  | "/tax-events"
  | "/quarantine"
  | "/diagnostics"
  | "/settings"
  | "/assistant";

export type SettingsMenuSection =
  | "privacy"
  | "display"
  | "security"
  | "backends"
  | "ai"
  | "data";

export type NativeMenuPayload =
  | { action: "lock-app" | "toggle-sensitive" }
  | { action: "sync-all-wallets" | "process-journals" }
  | { action: "open-settings"; section?: SettingsMenuSection | null }
  | { action: "navigate"; route?: AppRoutePath | null };

export const APP_ROUTE_PATHS: readonly AppRoutePath[] = [
  "/overview",
  "/transactions",
  "/reports",
  "/source-of-funds",
  "/connections",
  "/books",
  "/journals",
  "/tax-events",
  "/quarantine",
  "/diagnostics",
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
  runWalletSync: () => void;
  runJournalProcessing: () => void;
  addNotification: (notification: MenuIntentNotification) => void;
  emitSettingsSection: (section: string | null) => void;
}

export function dispatchMenuIntent(
  payload: NativeMenuPayload,
  deps: MenuIntentDeps,
): void {
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
      // Diagnostics stays reachable so support links keep working before
      // a workspace exists.
      if (!deps.hasWorkspace && payload.route !== "/diagnostics") return;
      deps.navigate({ to: payload.route });
      return;
    }
  }
}
