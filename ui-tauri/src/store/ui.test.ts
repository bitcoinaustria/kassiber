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

  it("does not persist notification progress to localStorage", () => {
    const state = {
      ...useUiStore.getState(),
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
  });

  it("normalizes persisted UI scale to the supported menu range", () => {
    expect(normalizeAppScale(0.93)).toBe(0.95);
    expect(normalizeAppScale(0.1)).toBe(MIN_APP_SCALE);
    expect(normalizeAppScale(2)).toBe(MAX_APP_SCALE);
    expect(normalizeAppScale("large")).toBe(DEFAULT_APP_SCALE);
  });
});
