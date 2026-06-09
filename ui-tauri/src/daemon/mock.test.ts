import { describe, expect, it } from "vitest";

import { fixtures } from "./fixtures";
import { mockDaemon } from "./mock";
import type { DaemonStreamRecord } from "./transport";

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

    const setDefault = await mockDaemon.invoke<{ default_backend: string }>({
      kind: "ui.backends.set_default",
      args: { name: "mock-extra" },
    });
    expect(setDefault.error).toBeUndefined();
    expect(setDefault.data?.default_backend).toBe("mock-extra");

    const relisted = await mockDaemon.invoke<{
      backends: Array<{ name: string; is_default?: boolean }>;
      summary: { default_backend: string | null };
    }>({ kind: "ui.backends.settings.list" });
    expect(relisted.data?.summary.default_backend).toBe("mock-extra");
    expect(
      relisted.data?.backends.find((row) => row.name === "mock-extra")
        ?.is_default,
    ).toBe(true);

    const deleted = await mockDaemon.invoke<{ deleted: boolean }>({
      kind: "ui.backends.delete",
      args: { name: "mock-extra" },
    });
    expect(deleted.data?.deleted).toBe(true);

    await mockDaemon.invoke({
      kind: "ui.backends.set_default",
      args: { name: "mempool" },
    });
  });
});

describe("mock daemon rate refresh", () => {
  it("updates the overview market rate from the latest quote path", async () => {
    const overview = fixtures["ui.overview.snapshot"] as {
      marketRate?: {
        rate?: number | null;
        fetchedAt?: string | null;
      };
      fiat?: {
        eurBalance?: number;
        eurUnrealized?: number;
      };
    };
    const previousMarketRate = overview.marketRate
      ? { ...overview.marketRate }
      : null;
    const previousFiat = overview.fiat ? { ...overview.fiat } : null;
    try {
      const before = await mockDaemon.invoke<{
        marketRate?: {
          rate?: number | null;
          fetchedAt?: string | null;
        };
      }>({ kind: "ui.overview.snapshot" });
      const previousRate = before.data?.marketRate?.rate ?? null;

      const refreshed = await mockDaemon.invoke<{
        pair: string;
        latest: Array<{ pair: string; mode?: string; samples?: number }>;
        marketRate?: {
          pair?: string | null;
          rate?: number | null;
          fetchedAt?: string | null;
          source?: string | null;
        } | null;
      }>({
        kind: "ui.rates.latest",
        args: { pair: "BTC-EUR" },
      });
      expect(refreshed.error).toBeUndefined();
      expect(refreshed.data?.pair).toBe("BTC-EUR");
      expect(refreshed.data?.latest[0]?.mode).toBe("latest_quote");
      expect(refreshed.data?.marketRate?.rate).not.toBe(previousRate);

      const after = await mockDaemon.invoke<{
        marketRate?: {
          pair?: string | null;
          rate?: number | null;
          fetchedAt?: string | null;
          source?: string | null;
        };
      }>({ kind: "ui.overview.snapshot" });

      expect(after.data?.marketRate?.pair).toBe("BTC-EUR");
      expect(after.data?.marketRate?.source).toBe("coinbase-exchange");
      expect(after.data?.marketRate?.rate).toBe(
        refreshed.data?.marketRate?.rate,
      );
      expect(Date.parse(after.data?.marketRate?.fetchedAt ?? "")).not.toBeNaN();
    } finally {
      if (overview.marketRate && previousMarketRate) {
        Object.assign(overview.marketRate, previousMarketRate);
      }
      if (overview.fiat && previousFiat) {
        Object.assign(overview.fiat, previousFiat);
      }
    }
  });

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

describe("mock daemon streams", () => {
  it("streams daemon-owned freshness progress", async () => {
    const records: DaemonStreamRecord[] = [];

    const envelope = await mockDaemon.stream<{ completed?: unknown[] }>(
      {
        kind: "ui.freshness.run",
        request_id: "freshness-mock-1",
        args: { all: true, rates: true, journals: true, run: true },
      },
      {
        onRecord(record) {
          records.push(record);
        },
      },
    );

    expect(envelope.kind).toBe("ui.freshness.run");
    expect(envelope.data?.completed?.length).toBeGreaterThan(0);
    expect(records.map((record) => record.kind)).toContain(
      "ui.freshness.run.progress",
    );
    expect(records[0]?.request_id).toBe("freshness-mock-1");
  });
});
