import { describe, expect, it } from "vitest";

import {
  DEFAULT_APP_SCALE,
  MAX_APP_SCALE,
  MIN_APP_SCALE,
  normalizeAppScale,
  uiStatePartialForStorage,
  useUiStore,
} from "./ui";

describe("UI persistence", () => {
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

  it("does not persist daemon logs or notification progress to localStorage", () => {
    const state = {
      ...useUiStore.getState(),
      logEntries: [
        {
          id: "log-1",
          createdAt: "2026-05-13T00:00:00Z",
          level: "error" as const,
          source: "daemon",
          message: "api_key=sk-localStorage-secret",
          details: { token: "btcpay-localStorage-secret" },
        },
      ],
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
    expect(encoded).not.toContain("sk-localStorage-secret");
    expect(encoded).not.toContain("btcpay-localStorage-secret");
    expect(encoded).not.toContain("secret-progress");
    expect(encoded).not.toContain("logEntries");
  });

  it("persists page workspace layouts separately from transient UI state", () => {
    const state = {
      ...useUiStore.getState(),
      pageWorkspaceLayouts: {
        "My Books:Tax:at:EUR::overview": {
          version: 1 as const,
          columns: 12,
          rowHeight: 96,
          items: [
            {
              id: "chart",
              widgetId: "treasury-chart",
              x: 0,
              y: 0,
              w: 8,
              h: 6,
              z: 2,
            },
          ],
        },
      },
    };

    const persisted = uiStatePartialForStorage(state);

    expect(persisted.pageWorkspaceLayouts).toEqual(state.pageWorkspaceLayouts);
  });

  it("normalizes persisted UI scale to the supported menu range", () => {
    expect(normalizeAppScale(0.93)).toBe(0.95);
    expect(normalizeAppScale(0.1)).toBe(MIN_APP_SCALE);
    expect(normalizeAppScale(2)).toBe(MAX_APP_SCALE);
    expect(normalizeAppScale("large")).toBe(DEFAULT_APP_SCALE);
  });
});
