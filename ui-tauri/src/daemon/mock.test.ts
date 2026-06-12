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
        args: { all: true, journals: true, run: true },
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

describe("mock daemon chat sessions", () => {
  it("lists seeded sessions with history state", async () => {
    const list = await mockDaemon.invoke<{
      sessions: Array<{ id: string; message_count: number }>;
      history_enabled: boolean;
    }>({ kind: "ui.chat.sessions.list" });
    expect(list.data?.history_enabled).toBe(true);
    expect(list.data?.sessions.length).toBeGreaterThanOrEqual(2);
    expect(list.data?.sessions[0]?.message_count).toBeGreaterThan(0);
  });

  it("errors on unknown session ids", async () => {
    const missing = await mockDaemon.invoke({
      kind: "ui.chat.sessions.get",
      args: { session_id: "nope" },
    });
    expect(missing.kind).toBe("error");
    expect(missing.error?.code).toBe("not_found");
  });

  it("persists an ai.chat exchange and round-trips it", async () => {
    const terminal = await mockDaemon.stream<{ session_id?: string | null }>(
      {
        kind: "ai.chat",
        request_id: "chat-mock-persist",
        args: {
          model: "mock-model",
          messages: [{ role: "user", content: "How many BTC moved today?" }],
          persist: "auto",
        },
      },
      { onRecord() {} },
    );
    const sessionId = terminal.data?.session_id;
    expect(typeof sessionId).toBe("string");

    const stored = await mockDaemon.invoke<{
      messages: Array<{ role: string; content: string }>;
    }>({ kind: "ui.chat.sessions.get", args: { session_id: sessionId } });
    expect(stored.data?.messages[0]?.content).toBe("How many BTC moved today?");
    expect(stored.data?.messages[1]?.role).toBe("assistant");

    const deleted = await mockDaemon.invoke<{ deleted?: string }>({
      kind: "ui.chat.sessions.delete",
      args: { session_id: sessionId },
    });
    expect(deleted.data?.deleted).toBe(sessionId);
  });

  it("keeps requests without persist intent ephemeral and honors off", async () => {
    const noIntent = await mockDaemon.stream<{ session_id?: string | null }>(
      {
        kind: "ai.chat",
        request_id: "chat-mock-ephemeral",
        args: {
          model: "mock-model",
          messages: [{ role: "user", content: "ephemeral question" }],
        },
      },
      { onRecord() {} },
    );
    expect(noIntent.data?.session_id ?? null).toBeNull();

    await mockDaemon.invoke({
      kind: "ui.chat.history.configure",
      args: { history: "off" },
    });
    const blocked = await mockDaemon.stream<{ session_id?: string | null }>(
      {
        kind: "ai.chat",
        request_id: "chat-mock-blocked",
        args: {
          model: "mock-model",
          messages: [{ role: "user", content: "blocked question" }],
          persist: "auto",
        },
      },
      { onRecord() {} },
    );
    expect(blocked.data?.session_id ?? null).toBeNull();

    const restored = await mockDaemon.invoke<{ history?: string }>({
      kind: "ui.chat.history.configure",
      args: { history: "auto" },
    });
    expect(restored.data?.history).toBe("auto");
  });
});

describe("mock daemon chat session fidelity", () => {
  it("fails ai.chat fast on unknown session ids like the real daemon", async () => {
    const records: DaemonStreamRecord[] = [];
    const envelope = await mockDaemon.stream(
      {
        kind: "ai.chat",
        request_id: "chat-mock-unknown-session",
        args: {
          model: "mock-model",
          messages: [{ role: "user", content: "hello" }],
          persist: "auto",
          session_id: "deleted-session",
        },
      },
      {
        onRecord(record) {
          records.push(record);
        },
      },
    );
    expect(envelope.kind).toBe("error");
    expect(envelope.error?.code).toBe("not_found");
    expect(records).toHaveLength(0);
  });

  it("rejects invalid history modes and unknown deletes", async () => {
    const invalid = await mockDaemon.invoke({
      kind: "ui.chat.history.configure",
      args: { history: "bogus" },
    });
    expect(invalid.error?.code).toBe("validation");

    const missing = await mockDaemon.invoke({
      kind: "ui.chat.sessions.delete",
      args: { session_id: "nope" },
    });
    expect(missing.error?.code).toBe("not_found");
  });
});
