import { describe, expect, it } from "vitest";

import {
  ASSISTANT_CHAT_PERMISSION_MODES_LIMIT,
  SIDE_PANEL_DEFAULT_WIDTH,
  SIDE_PANEL_MAX_WIDTH,
  SIDE_PANEL_MIN_WIDTH,
  DEFAULT_APP_SCALE,
  DEFAULT_ASSISTANT_TOOL_PROFILE,
  DEFAULT_THEME,
  MAX_APP_SCALE,
  MIN_APP_SCALE,
  normalizeAppScale,
  normalizeAnalysisNetwork,
  normalizeBookChartPeriods,
  migrateUiState,
  nextPrivacyMode,
  restoredPrivacy,
  normalizeAssistantChatPermissionModes,
  normalizeAssistantModelFavorites,
  normalizeSidePanelWidth,
  restoredSidePanelWidth,
  normalizeAssistantPermissionMode,
  normalizeAssistantProviderToolProfiles,
  uiStatePartialForStorage,
  useUiStore,
} from "./ui";

describe("UI persistence", () => {
  it("defaults new installs to dark mode", () => {
    expect(DEFAULT_THEME).toBe("dark");
    expect(useUiStore.getInitialState().theme).toBe("dark");
  });

  it("keeps native update consent and results out of renderer persistence", () => {
    useUiStore.getState().setAppUpdate({
      currentVersion: "0.22.55",
      latestVersion: "0.23.0",
      releaseUrl:
        "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.23.0",
      updateAvailable: true,
      prerelease: false,
      checkedAt: 1_784_688_800,
    });
    useUiStore.getState().setAutomaticUpdateChecks(false);

    const stored = uiStatePartialForStorage(useUiStore.getState());
    expect(stored).not.toHaveProperty("automaticUpdateChecks");
    expect(stored).not.toHaveProperty("appUpdate");
    expect(useUiStore.getState().appUpdate).toBeNull();

    useUiStore.getState().setAutomaticUpdateChecks(true);
    useUiStore.getState().setAppUpdate(null);
  });

  it("keeps desktop notifications off by default and stores only the boolean", () => {
    expect(useUiStore.getInitialState().desktopNotifications).toBe(false);

    useUiStore.getState().setDesktopNotifications(true);
    const stored = uiStatePartialForStorage(useUiStore.getState());
    expect(stored.desktopNotifications).toBe(true);
    // No delivery history, token, or destination travels with it.
    expect(Object.keys(stored).filter((key) => /notif/i.test(key))).toEqual([
      "desktopNotifications",
    ]);
    useUiStore.getState().setDesktopNotifications(false);
  });

  it("refuses a late update result after consent is disabled", () => {
    useUiStore.getState().setAutomaticUpdateChecks(false);
    useUiStore.getState().setAppUpdate({
      currentVersion: "0.22.55",
      latestVersion: "0.23.0",
      releaseUrl:
        "https://github.com/bitcoinaustria/kassiber/releases/tag/v0.23.0",
      updateAvailable: true,
      prerelease: false,
      checkedAt: 1_784_688_800,
    });

    expect(useUiStore.getState().appUpdate).toBeNull();
  });

  it("coalesces notifications with the same dedupe key", () => {
    useUiStore.setState({ notifications: [] });
    const first = useUiStore.getState().addNotification({
      title: "Connection refresh started",
      body: "Scanning",
      tone: "warning",
      dedupeKey: "wallet-sync",
      progress: { label: "Preparing wallet scan" },
    });
    const second = useUiStore.getState().addNotification({
      title: "Connection refresh finished",
      body: "2 sources current",
      tone: "success",
      dedupeKey: "wallet-sync",
    });

    expect(second).toBe(first);
    expect(useUiStore.getState().notifications).toHaveLength(1);
    expect(useUiStore.getState().notifications[0]).toMatchObject({
      id: first,
      title: "Connection refresh finished",
      body: "2 sources current",
      dedupeKey: "wallet-sync",
    });
    expect(useUiStore.getState().notifications[0].progress).toBeUndefined();
  });

  it("does not persist notification progress to localStorage", () => {
    const state = {
      ...useUiStore.getState(),
      theme: "dark" as const,
      clearClipboard: false,
      notifications: [
        {
          id: "notification-1",
          createdAt: "2026-05-13T00:00:00Z",
          title: "Sync",
          body: "Running",
          tone: "info" as const,
          progress: { label: "token=secret-progress" },
        },
      ],
    };

    const encoded = JSON.stringify(uiStatePartialForStorage(state));
    expect(encoded).not.toContain("secret-progress");
    expect(encoded).toContain('"theme":"dark"');
    expect(encoded).toContain('"clearClipboard":false');
  });

  it("keeps notifications in memory but never in kb.ui", () => {
    useUiStore.setState({ notifications: [] });
    useUiStore.getState().addNotification({
      title: "Book refresh needs attention",
      body: "1 needs attention",
      tone: "warning",
      details: ["Cold wallet: HTTP 502 from https://node.example:3002/api"],
      action: { label: "Retry", command: "refresh-book" },
    });

    expect(useUiStore.getState().notifications[0].details).toEqual([
      "Cold wallet: HTTP 502 from https://node.example:3002/api",
    ]);
    const encoded = JSON.stringify(
      uiStatePartialForStorage(useUiStore.getState()),
    );
    expect(encoded).not.toContain("node.example");
    expect(encoded).not.toContain('"details"');
    // Nothing of a notification survives a restart, its action least of all:
    // replayed later, it would run against whichever book is open then.
    expect(encoded).not.toContain('"command":"refresh-book"');
    expect(encoded).not.toContain("Book refresh needs attention");
  });

  it("drops stored notifications when migrating to v5", () => {
    const migrated = migrateUiState(
      {
        theme: "light",
        notifications: [
          {
            id: "old",
            title: "Book refresh failed",
            action: { label: "Retry", command: "refresh-book" },
          },
        ],
      },
      4,
    );
    expect(migrated).toMatchObject({ theme: "light" });
    expect((migrated as { notifications?: unknown }).notifications).toBeUndefined();
  });

  it("clears the replaced entry's details and action on a dedupe update", () => {
    useUiStore.setState({ notifications: [] });
    const { addNotification } = useUiStore.getState();
    addNotification({
      title: "Book refresh failed",
      body: "Electrum timeout",
      tone: "error",
      dedupeKey: "book-refresh",
      details: ["Cold wallet: timeout"],
      action: { label: "Retry", command: "refresh-book" },
    });
    addNotification({
      title: "Book refresh started",
      body: "Refreshing",
      tone: "warning",
      dedupeKey: "book-refresh",
    });

    const [entry] = useUiStore.getState().notifications;
    expect(entry.title).toBe("Book refresh started");
    expect(entry.details).toBeUndefined();
    expect(entry.action).toBeUndefined();
    expect(entry.repeatCount).toBeUndefined();
  });

  it("counts word-for-word repeats instead of adding rows", () => {
    useUiStore.setState({ notifications: [] });
    const { addNotification } = useUiStore.getState();
    const failure = {
      title: "Export failed",
      body: "Disk full",
      tone: "error" as const,
    };
    const first = addNotification(failure);
    expect(addNotification(failure)).toBe(first);
    addNotification(failure);
    addNotification({ ...failure, body: "Permission denied" });

    const notifications = useUiStore.getState().notifications;
    expect(notifications).toHaveLength(2);
    expect(notifications[1]).toMatchObject({ id: first, repeatCount: 3 });
    expect(notifications[0].repeatCount).toBeUndefined();

    // A keyed notification counts only when its text is unchanged.
    useUiStore.setState({ notifications: [] });
    const keyed = { ...failure, dedupeKey: "background-freshness" };
    addNotification(keyed);
    addNotification(keyed);
    expect(useUiStore.getState().notifications[0].repeatCount).toBe(2);
    addNotification({ ...keyed, title: "Book refresh needs attention" });
    expect(useUiStore.getState().notifications[0].repeatCount).toBeUndefined();
  });

  it("never counts a keyed running job as a repeat", () => {
    useUiStore.setState({ notifications: [] });
    const { addNotification } = useUiStore.getState();
    const running = {
      title: "Refreshing book",
      body: "Syncing wallets",
      tone: "info" as const,
      dedupeKey: "book-refresh",
      progress: { indeterminate: true },
    };
    addNotification(running);
    addNotification(running);
    addNotification(running);
    const { notifications } = useUiStore.getState();
    expect(notifications).toHaveLength(1);
    expect(notifications[0].repeatCount).toBeUndefined();
  });

  it("does not fold a running job into an identical finished one", () => {
    useUiStore.setState({ notifications: [] });
    const { addNotification } = useUiStore.getState();
    const notice = { title: "Import", body: "Working", tone: "info" as const };
    addNotification(notice);
    addNotification({ ...notice, progress: { indeterminate: true } });
    expect(useUiStore.getState().notifications).toHaveLength(2);
  });

  it("persists overview chart periods per book", () => {
    useUiStore.setState({ bookChartPeriods: {} });
    useUiStore.getState().setBookChartPeriod("db:/books/a.sqlite3", "4years");
    useUiStore.getState().setBookChartPeriod("db:/books/b.sqlite3", "2033");

    expect(useUiStore.getState().bookChartPeriods).toMatchObject({
      "db:/books/a.sqlite3": "4years",
      "db:/books/b.sqlite3": "2033",
    });

    const encoded = JSON.stringify(
      uiStatePartialForStorage(useUiStore.getState()),
    );
    expect(encoded).toContain('"bookChartPeriods"');
    expect(encoded).toContain('"db:/books/a.sqlite3":"4years"');
  });

  it("lands a stored retired range on the closest one offered", () => {
    expect(
      normalizeBookChartPeriods({
        a: "5years",
        b: "6months",
        c: "2033",
        d: "fortnight",
        e: 7,
      }),
    ).toEqual({ a: "4years", b: "1year", c: "2033" });
  });

  it("keeps active maintenance progress transient and clears by id", () => {
    const startedAt = "2026-06-06T10:00:00Z";
    useUiStore.getState().setActiveMaintenanceProgress({
      id: "book-refresh",
      title: "Refreshing book",
      body: "token=secret-progress",
      tone: "warning",
      progress: { value: 40, label: "token=secret-progress" },
      state: "running",
      startedAt,
      updatedAt: startedAt,
    });

    useUiStore.getState().clearActiveMaintenanceProgress("other-progress");
    expect(useUiStore.getState().activeMaintenanceProgress?.id).toBe(
      "book-refresh",
    );

    const encoded = JSON.stringify(
      uiStatePartialForStorage(useUiStore.getState()),
    );
    expect(encoded).not.toContain("secret-progress");

    useUiStore.getState().clearActiveMaintenanceProgress("book-refresh");
    expect(useUiStore.getState().activeMaintenanceProgress).toBeNull();
  });

  it("persists starred chat models as bare provider/model pairs", () => {
    useUiStore.setState({ assistantModelFavorites: [] });
    const { toggleAssistantModelFavorite } = useUiStore.getState();
    toggleAssistantModelFavorite({ provider: "ollama", model: "qwen3:8b" });
    toggleAssistantModelFavorite({ provider: "codex", model: "gpt-5.4" });
    toggleAssistantModelFavorite({ provider: "ollama", model: "qwen3:8b" });

    expect(useUiStore.getState().assistantModelFavorites).toEqual([
      { provider: "codex", model: "gpt-5.4" },
    ]);
    expect(
      uiStatePartialForStorage(useUiStore.getState()).assistantModelFavorites,
    ).toEqual([{ provider: "codex", model: "gpt-5.4" }]);
    useUiStore.setState({ assistantModelFavorites: [] });
  });

  it("drops malformed and duplicate restored favorites", () => {
    expect(
      normalizeAssistantModelFavorites([
        { provider: "codex", model: "gpt-5.4", base_url: "https://x" },
        { provider: "codex", model: "gpt-5.4" },
        { provider: "", model: "m" },
        { provider: "p", model: 3 },
        "codex:gpt",
        null,
      ]),
    ).toEqual([{ provider: "codex", model: "gpt-5.4" }]);
    expect(normalizeAssistantModelFavorites("nope")).toEqual([]);
  });

  it("normalizes persisted UI scale to the supported menu range", () => {
    expect(normalizeAppScale(0.93)).toBe(0.95);
    expect(normalizeAppScale(0.1)).toBe(MIN_APP_SCALE);
    expect(normalizeAppScale(2)).toBe(MAX_APP_SCALE);
    expect(normalizeAppScale("large")).toBe(DEFAULT_APP_SCALE);
  });
});

describe("analysis network values", () => {
  it("rejects unknown restored networks and preserves explicit valid contexts", () => {
    for (const network of ["main", "test", "signet", "regtest"]) {
      expect(normalizeAnalysisNetwork(network)).toBe(network);
    }
    for (const invalid of [undefined, null, "bitcoin", "https://example.invalid", {}]) {
      expect(normalizeAnalysisNetwork(invalid)).toBe("main");
    }
  });
});

describe("assistant panel preferences", () => {
  it("migrates the floating dock's auto-hide to the closed-panel bar", () => {
    // A user who had found the dock saw the parked handle: keep that quiet.
    expect(
      migrateUiState(
        { assistantDockAutoHide: true, assistantDockDiscovered: true, assistantDockPosition: "left" } as never,
        1,
      ),
    ).toMatchObject({ assistantClosedBar: "launcher", assistantDockPosition: "left" });
    // A missing discovery flag meant "veteran" in the old merge.
    expect(migrateUiState({ assistantDockAutoHide: true } as never, 1)).toMatchObject({
      assistantClosedBar: "launcher",
    });
    // Everyone else saw the composer, the old default, which v4 turns off.
    for (const legacy of [
      { assistantDockAutoHide: false, assistantDockDiscovered: true },
      { assistantDockAutoHide: true, assistantDockDiscovered: false },
      {},
    ]) {
      expect(migrateUiState(legacy as never, 1)).toMatchObject({
        assistantClosedBar: "hidden",
      });
    }
    const migrated = migrateUiState({ assistantDockAutoHide: true } as never, 1);
    expect(migrated).not.toHaveProperty("assistantDockAutoHide", true);
    // Already-migrated state is left alone.
    expect(migrateUiState({ assistantClosedBar: "hidden" } as never, 2)).toMatchObject({
      assistantClosedBar: "hidden",
    });
  });

  it("turns the old default closed bar off once", () => {
    expect(migrateUiState({ assistantClosedBar: "composer" } as never, 3)).toMatchObject({
      assistantClosedBar: "hidden",
    });
    // The launcher was only ever chosen, so it stays.
    expect(migrateUiState({ assistantClosedBar: "launcher" } as never, 3)).toMatchObject({
      assistantClosedBar: "launcher",
    });
    // A composer picked after v4 is a choice.
    expect(migrateUiState({ assistantClosedBar: "composer" } as never, 4)).toMatchObject({
      assistantClosedBar: "composer",
    });
  });

  it("persists the panel state and clamps a stored width", () => {
    useUiStore.getState().setAssistantPanelOpen(true);
    useUiStore.getState().setSidePanelWidth(10_000);
    const stored = uiStatePartialForStorage(useUiStore.getState());
    expect(stored).toMatchObject({
      assistantPanelOpen: true,
      sidePanelWidth: SIDE_PANEL_MAX_WIDTH,
    });
    // The focus request is a transient signal, not a preference.
    expect(stored).not.toHaveProperty("assistantPanelFocusRequest");
    expect(normalizeSidePanelWidth(12)).toBe(SIDE_PANEL_MIN_WIDTH);
    expect(normalizeSidePanelWidth("wide")).toBe(SIDE_PANEL_DEFAULT_WIDTH);
    useUiStore.getState().setAssistantPanelOpen(false);
    useUiStore.getState().setSidePanelWidth(SIDE_PANEL_DEFAULT_WIDTH);
  });

  it("restores one panel width from the two widths it used to remember", () => {
    // The wider of the old assistant and details widths carries over.
    expect(restoredSidePanelWidth({ assistantPanelWidth: 596, detailPanelWidth: 903 })).toBe(903);
    expect(restoredSidePanelWidth({ assistantPanelWidth: 620 })).toBe(620);
    expect(restoredSidePanelWidth({ detailPanelWidth: "wide" })).toBeUndefined();
    // A saved shared width wins over the old ones.
    expect(
      restoredSidePanelWidth({ sidePanelWidth: 700, detailPanelWidth: 903 }),
    ).toBe(700);
    expect(restoredSidePanelWidth({})).toBeUndefined();
  });

  it("focuses the composer when the toggle opens the panel", () => {
    const before = useUiStore.getState().assistantPanelFocusRequest;
    useUiStore.getState().toggleAssistantPanel();
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
    expect(useUiStore.getState().assistantPanelFocusRequest).toBe(before + 1);
    expect(useUiStore.getState().rightPanelOpen).toBe(true);
    // Toggling again hides the panel; the assistant stays one of its tabs.
    useUiStore.getState().toggleAssistantPanel();
    expect(useUiStore.getState().rightPanelOpen).toBe(false);
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
    expect(useUiStore.getState().assistantPanelFocusRequest).toBe(before + 1);
    useUiStore.getState().setAssistantPanelOpen(false);
  });

  it("shows and hides the right panel without touching its tabs", () => {
    useUiStore.getState().setAssistantPanelOpen(true);
    useUiStore.getState().setRightPanelMaximized(true);
    useUiStore.getState().toggleRightPanel();
    expect(useUiStore.getState()).toMatchObject({
      rightPanelOpen: false,
      rightPanelMaximized: false,
      assistantPanelOpen: true,
    });
    const reveal = useUiStore.getState().assistantPanelRevealRequest;
    useUiStore.getState().toggleRightPanel();
    expect(useUiStore.getState().rightPanelOpen).toBe(true);
    // Showing it returns with whichever tab was in front.
    expect(useUiStore.getState().assistantPanelRevealRequest).toBe(reveal);
    // A page holding the column is asked to give it back.
    useUiStore.getState().setAssistantColumnClaimed(true);
    useUiStore.getState().toggleRightPanel();
    expect(useUiStore.getState().assistantPanelRevealRequest).toBe(reveal + 1);
    useUiStore.getState().setAssistantColumnClaimed(false);
    // Closing the assistant tab leaves the panel (and its launcher) open.
    useUiStore.getState().setAssistantPanelOpen(false);
    expect(useUiStore.getState().rightPanelOpen).toBe(true);
    useUiStore.getState().setRightPanelOpen(false);
  });
});

describe("assistant permission preferences", () => {
  it("starts new chats in Auto and keeps a bounded mode per saved chat", () => {
    expect(useUiStore.getInitialState().assistantDefaultPermissionMode).toBe("auto");
    expect(normalizeAssistantPermissionMode("root")).toBeNull();
    expect(
      normalizeAssistantChatPermissionModes({ a: "auto", b: "sudo", "": "approval", c: 3 }),
    ).toEqual({ a: "auto" });

    const store = useUiStore.getState();
    for (let index = 0; index <= ASSISTANT_CHAT_PERMISSION_MODES_LIMIT; index += 1) {
      store.rememberAssistantChatPermissionMode(`s-${index}`, "approval");
    }
    // Using the oldest chat again keeps it; the next oldest goes.
    store.rememberAssistantChatPermissionMode("s-1", "full_access");
    store.rememberAssistantChatPermissionMode("s-new", "auto");
    const modes = useUiStore.getState().assistantChatPermissionModes;
    expect(Object.keys(modes)).toHaveLength(ASSISTANT_CHAT_PERMISSION_MODES_LIMIT);
    expect(modes["s-1"]).toBe("full_access");
    expect(modes["s-2"]).toBeUndefined();
    expect(uiStatePartialForStorage(useUiStore.getState()).assistantChatPermissionModes).toBe(modes);

    store.forgetAssistantChatPermissionModes("s-1");
    expect(useUiStore.getState().assistantChatPermissionModes["s-1"]).toBeUndefined();
    store.forgetAssistantChatPermissionModes();
    expect(useUiStore.getState().assistantChatPermissionModes).toEqual({});
  });
});

describe("assistant tool set preferences", () => {
  it("gives every provider all tools unless the user picked fewer", () => {
    expect(DEFAULT_ASSISTANT_TOOL_PROFILE).toBe("full");
    expect(useUiStore.getInitialState().assistantProviderToolProfiles).toEqual({});
    expect(
      normalizeAssistantProviderToolProfiles({ ollama: "core", codex: "huge", "": "scoped" }),
    ).toEqual({ ollama: "core" });
    const store = useUiStore.getState();
    store.setAssistantProviderToolProfile("ollama", "scoped");
    expect(
      uiStatePartialForStorage(useUiStore.getState()).assistantProviderToolProfiles,
    ).toEqual({ ollama: "scoped" });
    store.setAssistantProviderToolProfile("ollama", null);
    expect(useUiStore.getState().assistantProviderToolProfiles).toEqual({});
  });
});

describe("privacy mode", () => {
  it("migrates the old hide-sensitive switch to hiding amounts", () => {
    expect(migrateUiState({ hideSensitive: true } as never, 2)).toMatchObject({
      privacyMode: "hideAmounts",
    });
    expect(migrateUiState({ hideSensitive: false } as never, 2)).toMatchObject({
      privacyMode: "normal",
    });
    expect(migrateUiState({} as never, 1)).toMatchObject({ privacyMode: "normal" });
    // Already-migrated state is left alone.
    expect(migrateUiState({ privacyMode: "hideAll" } as never, 3)).toMatchObject({
      privacyMode: "hideAll",
    });
  });

  it("persists the mode, not the derived flag", () => {
    useUiStore.getState().setPrivacyMode("hideAll");
    const stored = uiStatePartialForStorage(useUiStore.getState());
    expect(stored).toMatchObject({ privacyMode: "hideAll", hideAmountsOnUnlock: false });
    expect(stored).not.toHaveProperty("hideSensitive");
    useUiStore.getState().setPrivacyMode("normal");
  });

  it("cycles normal → hide amounts → hide all → normal, keeping hideSensitive in sync", () => {
    const store = useUiStore.getState;
    store().setPrivacyMode("normal");
    expect(store().hideSensitive).toBe(false);
    store().cyclePrivacyMode();
    expect(store()).toMatchObject({ privacyMode: "hideAmounts", hideSensitive: true });
    store().cyclePrivacyMode();
    expect(store()).toMatchObject({ privacyMode: "hideAll", hideSensitive: true });
    store().cyclePrivacyMode();
    expect(store()).toMatchObject({ privacyMode: "normal", hideSensitive: false });
    expect(nextPrivacyMode("hideAll")).toBe("normal");
  });

  it("keeps hide all when a boolean caller asks to hide", () => {
    const store = useUiStore.getState;
    store().setPrivacyMode("hideAll");
    store().setHideSensitive(true);
    expect(store().privacyMode).toBe("hideAll");
    store().setHideSensitive(false);
    expect(store().privacyMode).toBe("normal");
    store().setHideSensitive(true);
    expect(store().privacyMode).toBe("hideAmounts");
    store().setPrivacyMode("normal");
  });

  it("raises the mode on unlock only when the user asked for it", () => {
    const store = useUiStore.getState;
    store().setPrivacyMode("normal");
    store().enforceUnlockPrivacy();
    expect(store().privacyMode).toBe("normal");

    store().setHideAmountsOnUnlock(true);
    store().enforceUnlockPrivacy();
    expect(store().privacyMode).toBe("hideAmounts");
    // Never lowers a stricter choice.
    store().setPrivacyMode("hideAll");
    store().enforceUnlockPrivacy();
    expect(store().privacyMode).toBe("hideAll");

    store().setHideAmountsOnUnlock(false);
    store().setPrivacyMode("normal");
  });

  it("starts hidden at launch when hide-on-unlock is on", () => {
    const current = { privacyMode: "normal" } as const;
    expect(
      restoredPrivacy({ privacyMode: "normal", hideAmountsOnUnlock: true }, current),
    ).toMatchObject({ privacyMode: "hideAmounts", hideSensitive: true });
    expect(restoredPrivacy({ privacyMode: "hideAll" }, current)).toMatchObject({
      privacyMode: "hideAll",
      hideSensitive: true,
      hideAmountsOnUnlock: false,
    });
    expect(restoredPrivacy({ privacyMode: "bogus" } as never, current)).toMatchObject({
      privacyMode: "normal",
      hideSensitive: false,
    });
  });
});
