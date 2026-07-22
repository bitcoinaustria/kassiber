import { describe, expect, it } from "vitest";

import {
  DEFAULT_APP_SCALE,
  DEFAULT_THEME,
  MAX_APP_SCALE,
  MIN_APP_SCALE,
  normalizeAppScale,
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

  it("persists overview chart periods per book", () => {
    useUiStore.setState({ bookChartPeriods: {} });
    useUiStore.getState().setBookChartPeriod("db:/books/a.sqlite3", "5years");
    useUiStore.getState().setBookChartPeriod("db:/books/b.sqlite3", "30days");

    expect(useUiStore.getState().bookChartPeriods).toMatchObject({
      "db:/books/a.sqlite3": "5years",
      "db:/books/b.sqlite3": "30days",
    });

    const encoded = JSON.stringify(
      uiStatePartialForStorage(useUiStore.getState()),
    );
    expect(encoded).toContain('"bookChartPeriods"');
    expect(encoded).toContain('"db:/books/a.sqlite3":"5years"');
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

  it("normalizes persisted UI scale to the supported menu range", () => {
    expect(normalizeAppScale(0.93)).toBe(0.95);
    expect(normalizeAppScale(0.1)).toBe(MIN_APP_SCALE);
    expect(normalizeAppScale(2)).toBe(MAX_APP_SCALE);
    expect(normalizeAppScale("large")).toBe(DEFAULT_APP_SCALE);
  });
});
