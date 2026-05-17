import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  clearAppLogRecords,
  emitAppLog,
  exportLogRecords,
  formatLogRecord,
  getAppLogRecords,
  logFilename,
  setAppLogSubscriptionLevel,
  stableMaskedValue,
  type AppLogRecord,
} from "./appLogs";

function record(fields: AppLogRecord["fields"] = {}): Omit<AppLogRecord, "id" | "ts"> {
  return {
    level: "info",
    module: "daemon.transport",
    file: "daemon/transport.ts",
    line: 42,
    msg: "Daemon invoke finished",
    fields,
  };
}

describe("typed app logs", () => {
  beforeEach(() => {
    const storage = new Map<string, string>();
    vi.stubGlobal("localStorage", {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => {
        storage.set(key, value);
      },
      removeItem: (key: string) => {
        storage.delete(key);
      },
      clear: () => {
        storage.clear();
      },
    });
    clearAppLogRecords();
    setAppLogSubscriptionLevel("info");
  });

  it("changes emission at the subscription level", () => {
    emitAppLog({ ...record(), level: "debug" });
    expect(getAppLogRecords()).toHaveLength(0);

    setAppLogSubscriptionLevel("debug");
    emitAppLog({ ...record(), level: "debug" });
    expect(getAppLogRecords()).toHaveLength(1);
  });

  it("masks sensitive typed fields without changing the message", () => {
    const emitted = emitAppLog(
      record({
        address: { type: "address", value: "bc1qexample000000000000f3xy" },
        descriptor_value: {
          type: "descriptor",
          value: "wpkh([abcd1234/84h/0h/0h]xpub661MyMwAqRbcFsecret/0/*)",
        },
        label: { type: "label", value: "Treasury hot wallet" },
        data_root: { type: "path", value: "/Users/dev/.kassiber/data" },
        amount: { type: "amount", value: "1.234 BTC" },
      }),
    );
    expect(emitted).not.toBeNull();

    const redacted = formatLogRecord(emitted!, {
      redacted: true,
      maskAmounts: false,
    });
    expect(redacted).toContain("Daemon invoke finished");
    expect(redacted).toContain("bc1qe...f3xy");
    expect(redacted).toContain("wallet#");
    expect(redacted).toContain("~/.../data");
    expect(redacted).toContain("1.234 BTC");
    expect(redacted).not.toContain("xpub");
    expect(redacted).not.toContain("descriptor");
    expect(redacted).not.toContain("/Users/");
    expect(redacted).not.toContain("Treasury hot wallet");

    expect(stableMaskedValue({ type: "label", value: "Treasury hot wallet" })).toBe(
      stableMaskedValue({ type: "label", value: "Treasury hot wallet" }),
    );
  });

  it("exports a self-describing markdown snapshot and watermarks raw output", () => {
    const emitted = emitAppLog(
      record({
        api_key: { type: "api_key", value: "sk-local-secret" },
        email: { type: "email", value: "dev@example.test" },
        onion: { type: "onion", value: "abcdefg123456789.onion" },
      }),
    );
    const redacted = exportLogRecords([emitted!], "md", {
      redacted: true,
      header: {
        appVersion: "0.22.0 (abc1234)",
        os: "macOS",
        timeRange: "2026-05-17T18:00:00Z to 2026-05-17T18:01:00Z",
        activeFilter: "subscription>=info, module=all, search=none",
        redaction: "redacted",
        generatedAt: "2026-05-17T18:08:00Z",
      },
    });
    expect(redacted).toContain("# Kassiber log snapshot");
    expect(redacted).toContain("App version: 0.22.0");
    expect(redacted).toContain("```log");
    expect(redacted).not.toContain("sk-local-secret");
    expect(redacted).not.toContain("@");
    expect(redacted).not.toContain(".onion");

    const raw = exportLogRecords([emitted!], "jsonl", {
      redacted: false,
      header: {
        appVersion: "0.22.0",
        os: "macOS",
        timeRange: "all",
        activeFilter: "none",
        redaction: "raw",
      },
    });
    expect(raw).toContain("RAW EXPORT");
    expect(raw).toContain("sk-local-secret");
  });

  it("encodes redaction state in filenames", () => {
    expect(logFilename("md", "redacted", new Date("2026-05-17T18:08:00Z"))).toBe(
      "kassiber-2026-05-17T18-08Z-redacted.md",
    );
  });
});
