import { describe, expect, it } from "vitest";

import { mockDaemon } from "./mock";

describe("mock daemon backend settings", () => {
  it("supports settings list and CRUD for demo mode", async () => {
    const list = await mockDaemon.invoke<{
      backends: Array<{ name: string }>;
    }>({ kind: "ui.backends.settings.list" });
    expect(list.data?.backends.length).toBeGreaterThan(0);

    const created = await mockDaemon.invoke<{ name: string }>({
      kind: "ui.backends.create",
      args: {
        name: "mock-extra",
        kind: "esplora",
        chain: "bitcoin",
        network: "main",
        url: "https://example.invalid/api",
        auth_header: "Bearer demo",
      },
    });
    expect(created.error).toBeUndefined();
    expect(created.data?.name).toBe("mock-extra");

    const updated = await mockDaemon.invoke<{
      display_name?: string;
      has_auth_header?: boolean;
      has_username?: boolean;
    }>({
      kind: "ui.backends.update",
      args: {
        name: "mock-extra",
        config: { display_name: "Demo endpoint", username: "demo" },
        clear: ["auth_header"],
      },
    });
    expect(updated.data?.display_name).toBe("Demo endpoint");
    expect(updated.data?.has_auth_header).toBe(false);
    expect(updated.data?.has_username).toBe(true);

    const deleted = await mockDaemon.invoke<{ deleted: boolean }>({
      kind: "ui.backends.delete",
      args: { name: "mock-extra" },
    });
    expect(deleted.data?.deleted).toBe(true);
  });
});

describe("mock daemon rate refresh", () => {
  it("updates the overview market rate sync timestamp when rates rebuild", async () => {
    try {
      const refreshed = await mockDaemon.invoke<{
        pair: string;
        sync: Array<{ pair: string }>;
      }>({
        kind: "ui.rates.rebuild",
        args: { pair: "BTC-CHF", source: "coinbase-exchange" },
      });
      expect(refreshed.error).toBeUndefined();
      expect(refreshed.data?.pair).toBe("BTC-CHF");

      const after = await mockDaemon.invoke<{
        marketRate?: {
          pair?: string | null;
          fetchedAt?: string | null;
          source?: string | null;
        };
      }>({ kind: "ui.overview.snapshot" });

      expect(after.data?.marketRate?.pair).toBe("BTC-CHF");
      expect(after.data?.marketRate?.source).toBe("coinbase-exchange");
      expect(Date.parse(after.data?.marketRate?.fetchedAt ?? "")).not.toBeNaN();
    } finally {
      await mockDaemon.invoke({
        kind: "ui.rates.rebuild",
        args: { pair: "BTC-EUR", source: "coinbase-exchange" },
      });
    }
  });
});
