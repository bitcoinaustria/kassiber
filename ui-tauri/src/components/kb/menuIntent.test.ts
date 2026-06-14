import { describe, expect, it, vi } from "vitest";

import {
  dispatchMenuIntent,
  isAppRoutePath,
  type MenuIntentDeps,
} from "./menuIntent";

function makeDeps(overrides: Partial<MenuIntentDeps> = {}): MenuIntentDeps {
  return {
    hasWorkspace: true,
    aiFeaturesEnabled: true,
    hideSensitive: false,
    navigate: vi.fn(),
    lockApp: vi.fn(),
    setHideSensitive: vi.fn(),
    decreaseAppScale: vi.fn(),
    increaseAppScale: vi.fn(),
    resetAppScale: vi.fn(),
    runAddWalletConnection: vi.fn(),
    runWalletSync: vi.fn(),
    runJournalProcessing: vi.fn(),
    addNotification: vi.fn(),
    emitSettingsSection: vi.fn(),
    ...overrides,
  };
}

describe("isAppRoutePath", () => {
  it("accepts every documented top-level route", () => {
    for (const route of [
      "/overview",
      "/transactions",
      "/reports",
      "/source-of-funds",
      "/connections",
      "/books",
      "/journals",
      "/quarantine",
      "/logs",
      "/settings",
      "/assistant",
    ]) {
      expect(isAppRoutePath(route)).toBe(true);
    }
  });

  it("rejects nested paths, query strings, and non-strings", () => {
    expect(isAppRoutePath("/transactions/abc")).toBe(false);
    expect(isAppRoutePath("/overview?focus=1")).toBe(false);
    expect(isAppRoutePath("")).toBe(false);
    expect(isAppRoutePath(null)).toBe(false);
    expect(isAppRoutePath(undefined)).toBe(false);
    expect(isAppRoutePath(42)).toBe(false);
  });
});

describe("dispatchMenuIntent — workspace gating", () => {
  // The native menu greys out workspace-required items via set_menu_state,
  // but `kassiber://lock` and `kassiber://transactions` deep links bypass
  // that. The dispatcher is the single chokepoint that has to mirror the
  // menu's enabled/disabled state — these tests pin that mirror.
  it("drops lock-app when no workspace is open", () => {
    const deps = makeDeps({ hasWorkspace: false });
    dispatchMenuIntent({ action: "lock-app" }, deps);
    expect(deps.lockApp).not.toHaveBeenCalled();
  });

  it("calls lockApp when a workspace is open", () => {
    const deps = makeDeps();
    dispatchMenuIntent({ action: "lock-app" }, deps);
    expect(deps.lockApp).toHaveBeenCalledTimes(1);
  });

  it("drops navigate when no workspace is open", () => {
    const deps = makeDeps({ hasWorkspace: false });
    dispatchMenuIntent(
      { action: "navigate", route: "/transactions" },
      deps,
    );
    expect(deps.navigate).not.toHaveBeenCalled();
    dispatchMenuIntent(
      { action: "navigate", route: "/logs" },
      deps,
    );
    expect(deps.navigate).not.toHaveBeenCalled();
  });
});

describe("dispatchMenuIntent — AI route fallback", () => {
  // When AI features are off, navigating to /assistant should redirect to
  // Settings → AI section with an explanatory notification, NOT silently
  // drop the action. Symmetric with the route guard at routeTree:assistant.
  it("redirects /assistant to Settings#ai when AI features are off", () => {
    const deps = makeDeps({ aiFeaturesEnabled: false });
    dispatchMenuIntent(
      { action: "navigate", route: "/assistant" },
      deps,
    );
    expect(deps.addNotification).toHaveBeenCalledWith(
      expect.objectContaining({ tone: "info" }),
    );
    expect(deps.navigate).toHaveBeenCalledWith({
      to: "/settings",
      hash: "ai",
    });
  });

  it("navigates to /assistant when AI features are on", () => {
    const deps = makeDeps({ aiFeaturesEnabled: true });
    dispatchMenuIntent(
      { action: "navigate", route: "/assistant" },
      deps,
    );
    expect(deps.navigate).toHaveBeenCalledWith({ to: "/assistant" });
    expect(deps.addNotification).not.toHaveBeenCalled();
  });
});

describe("dispatchMenuIntent — open-settings re-fires the section event", () => {
  // SettingsScreen listens for kassiber:settings-section so a repeat menu
  // click on the same hash re-opens the panel even when the URL didn't
  // change. The dispatcher always emits, even with a null section, so the
  // listener gets a consistent signal.
  it("navigates and emits the section side effect", () => {
    const deps = makeDeps();
    dispatchMenuIntent(
      { action: "open-settings", section: "bitcoin" },
      deps,
    );
    expect(deps.navigate).toHaveBeenCalledWith({
      to: "/settings",
      hash: "bitcoin",
    });
    expect(deps.emitSettingsSection).toHaveBeenCalledWith("bitcoin");
  });

  it("emits null when no section is provided", () => {
    const deps = makeDeps();
    dispatchMenuIntent({ action: "open-settings" }, deps);
    expect(deps.emitSettingsSection).toHaveBeenCalledWith(null);
  });
});

describe("dispatchMenuIntent — scope filter", () => {
  // The two listeners (RootIntentListener + AppShell's) split the surface
  // strictly so neither double-handles a sensitive toggle. Pin the split.
  it("global scope drops workspace actions", () => {
    const deps = makeDeps();
    dispatchMenuIntent({ action: "lock-app" }, deps, "global");
    dispatchMenuIntent({ action: "add-wallet" }, deps, "global");
    dispatchMenuIntent({ action: "sync-all-wallets" }, deps, "global");
    dispatchMenuIntent({ action: "process-journals" }, deps, "global");
    expect(deps.lockApp).not.toHaveBeenCalled();
    expect(deps.runAddWalletConnection).not.toHaveBeenCalled();
    expect(deps.runWalletSync).not.toHaveBeenCalled();
    expect(deps.runJournalProcessing).not.toHaveBeenCalled();
  });

  it("global scope handles route navigation, settings, and toggle", () => {
    const deps = makeDeps();
    dispatchMenuIntent(
      { action: "navigate", route: "/transactions" },
      deps,
      "global",
    );
    dispatchMenuIntent(
      { action: "open-settings", section: "privacy" },
      deps,
      "global",
    );
    dispatchMenuIntent({ action: "toggle-sensitive" }, deps, "global");
    dispatchMenuIntent({ action: "ui-scale-increase" }, deps, "global");
    expect(deps.navigate).toHaveBeenCalledTimes(2); // navigate + open-settings
    expect(deps.emitSettingsSection).toHaveBeenCalledWith("privacy");
    expect(deps.setHideSensitive).toHaveBeenCalledTimes(1);
    expect(deps.increaseAppScale).toHaveBeenCalledTimes(1);
  });

  it("workspace scope drops global actions", () => {
    const deps = makeDeps();
    dispatchMenuIntent(
      { action: "navigate", route: "/transactions" },
      deps,
      "workspace",
    );
    dispatchMenuIntent({ action: "open-settings" }, deps, "workspace");
    dispatchMenuIntent({ action: "toggle-sensitive" }, deps, "workspace");
    dispatchMenuIntent({ action: "ui-scale-reset" }, deps, "workspace");
    expect(deps.navigate).not.toHaveBeenCalled();
    expect(deps.emitSettingsSection).not.toHaveBeenCalled();
    expect(deps.setHideSensitive).not.toHaveBeenCalled();
    expect(deps.resetAppScale).not.toHaveBeenCalled();
  });

  it("workspace scope handles lock and workflow actions", () => {
    const deps = makeDeps();
    dispatchMenuIntent({ action: "lock-app" }, deps, "workspace");
    dispatchMenuIntent({ action: "add-wallet" }, deps, "workspace");
    dispatchMenuIntent({ action: "sync-all-wallets" }, deps, "workspace");
    dispatchMenuIntent({ action: "process-journals" }, deps, "workspace");
    expect(deps.lockApp).toHaveBeenCalledTimes(1);
    expect(deps.runAddWalletConnection).toHaveBeenCalledTimes(1);
    expect(deps.runWalletSync).toHaveBeenCalledTimes(1);
    expect(deps.runJournalProcessing).toHaveBeenCalledTimes(1);
  });
});

describe("dispatchMenuIntent — direct actions", () => {
  it("toggle-sensitive flips hideSensitive", () => {
    const offDeps = makeDeps({ hideSensitive: false });
    dispatchMenuIntent({ action: "toggle-sensitive" }, offDeps);
    expect(offDeps.setHideSensitive).toHaveBeenCalledWith(true);

    const onDeps = makeDeps({ hideSensitive: true });
    dispatchMenuIntent({ action: "toggle-sensitive" }, onDeps);
    expect(onDeps.setHideSensitive).toHaveBeenCalledWith(false);
  });

  it("routes UI scale actions to the app scale controls", () => {
    const deps = makeDeps();
    dispatchMenuIntent({ action: "ui-scale-decrease" }, deps);
    dispatchMenuIntent({ action: "ui-scale-increase" }, deps);
    dispatchMenuIntent({ action: "ui-scale-reset" }, deps);

    expect(deps.decreaseAppScale).toHaveBeenCalledTimes(1);
    expect(deps.increaseAppScale).toHaveBeenCalledTimes(1);
    expect(deps.resetAppScale).toHaveBeenCalledTimes(1);
  });

  it("add-wallet routes to the wallet connection runner", () => {
    const deps = makeDeps();
    dispatchMenuIntent({ action: "add-wallet" }, deps);
    expect(deps.runAddWalletConnection).toHaveBeenCalledTimes(1);
    expect(deps.runWalletSync).not.toHaveBeenCalled();
    expect(deps.runJournalProcessing).not.toHaveBeenCalled();
  });

  it("sync-all-wallets routes to the wallet sync runner", () => {
    const deps = makeDeps();
    dispatchMenuIntent({ action: "sync-all-wallets" }, deps);
    expect(deps.runWalletSync).toHaveBeenCalledTimes(1);
    expect(deps.runJournalProcessing).not.toHaveBeenCalled();
  });

  it("process-journals routes to the journal processor runner", () => {
    const deps = makeDeps();
    dispatchMenuIntent({ action: "process-journals" }, deps);
    expect(deps.runJournalProcessing).toHaveBeenCalledTimes(1);
    expect(deps.runWalletSync).not.toHaveBeenCalled();
  });

  it("delegates workflow workspace gating to the runners", () => {
    // Sanity check on the comment in dispatchMenuIntent: workflow actions
    // are dispatched even without a workspace; the runners are responsible
    // for redirecting to / via ensureWorkspaceForMenuAction. If the
    // dispatcher ever starts gating workflows itself, this test should be
    // updated alongside that change.
    const deps = makeDeps({ hasWorkspace: false });
    dispatchMenuIntent({ action: "add-wallet" }, deps);
    dispatchMenuIntent({ action: "sync-all-wallets" }, deps);
    dispatchMenuIntent({ action: "process-journals" }, deps);
    expect(deps.runAddWalletConnection).toHaveBeenCalledTimes(1);
    expect(deps.runWalletSync).toHaveBeenCalledTimes(1);
    expect(deps.runJournalProcessing).toHaveBeenCalledTimes(1);
  });

  it("ignores navigate with an unknown route", () => {
    const deps = makeDeps();
    // Simulating a malformed deep link / Rust-side bug where the route
    // doesn't match any AppRoutePath. Drop silently rather than crash.
    dispatchMenuIntent(
      {
        action: "navigate",
        route: "/wallet-of-satoshi" as unknown as never,
      },
      deps,
    );
    expect(deps.navigate).not.toHaveBeenCalled();
  });
});
