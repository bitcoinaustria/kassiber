import { describe, expect, it } from "vitest";

import { uiStatePartialForStorage, useUiStore } from "./ui";

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
});
