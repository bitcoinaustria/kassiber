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

  it("normalizes persisted UI scale to the supported menu range", () => {
    expect(normalizeAppScale(0.93)).toBe(0.95);
    expect(normalizeAppScale(0.1)).toBe(MIN_APP_SCALE);
    expect(normalizeAppScale(2)).toBe(MAX_APP_SCALE);
    expect(normalizeAppScale("large")).toBe(DEFAULT_APP_SCALE);
  });
});
