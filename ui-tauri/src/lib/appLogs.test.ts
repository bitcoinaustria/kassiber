import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  APP_LOG_MAX_BYTES,
  APP_LOG_MAX_RECORDS,
  clearAppLogRecords,
  emitAppLog,
  exportLogRecords,
  exportSupportBundleRecords,
  formatLogRecord,
  getAppLogBufferSize,
  getAppLogRecords,
  logFilename,
  setAppLogSubscriptionLevel,
  stableMaskedValue,
  supportBundleFilename,
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
    expect(redacted).toContain("address=bc1qe...f3xy");
    expect(redacted).toContain("wallet_material=wallet#");
    expect(redacted).toContain("label=wallet#");
    expect(redacted).toContain("path=~/.../data");
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

  it("keeps a redaction backstop for message and text fields", () => {
    const emitted = emitAppLog({
      ...record({
        detail: {
          type: "text",
          value: "Bearer sk-local-secret and descriptor=wpkh([abcd]xpub661MyMwAqRbcFsecret/0/*)",
        },
      }),
      msg: "failed with api_key=sk-local-secret",
    });
    expect(emitted).not.toBeNull();

    const redacted = formatLogRecord(emitted!, {
      redacted: true,
      maskAmounts: false,
    });
    expect(redacted).toContain("api_key=[redacted]");
    expect(redacted).toContain("Bearer [redacted]");
    expect(redacted).toContain("descriptor=[redacted]");
    expect(redacted).not.toContain("sk-local-secret");
    expect(redacted).not.toContain("xpub");
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
    expect(supportBundleFilename(new Date("2026-05-17T18:08:00Z"))).toBe(
      "kassiber-support-2026-05-17T18-08Z.support.jsonl",
    );
  });

  it("exports public-safe support bundles with failure context and AI provenance", () => {
    const txid = "a".repeat(64);
    const address = "bc1qexample000000000000000000000000000000f3xy";
    const descriptor = "wpkh([abcd1234/84h/0h/0h]xpub661MyMwAqRbcFsecret/0/*)";
    const url = "https://user:secret@example.test/wallet?api_key=sk-url-secret";
    const localPath = "/Users/dev/.kassiber/data/kassiber.sqlite";
    const records: AppLogRecord[] = [
      {
        ...record({
          request_id: { type: "text", value: "req-1" },
          trace_id: { type: "text", value: "req-1" },
          descriptor_value: { type: "descriptor", value: descriptor },
          backend_url: { type: "url", value: url },
          email: { type: "email", value: "dev@example.test" },
          data_root: { type: "path", value: localPath },
          amount: { type: "amount", value: "1.234 BTC" },
        }),
        id: "log-1",
        ts: "2026-05-17T18:00:00.000Z",
      },
      {
        id: "log-2",
        ts: "2026-05-17T18:00:01.000Z",
        level: "trace",
        module: "daemon:bridge",
        file: "daemon/transport.ts",
        line: 0,
        msg: "Daemon stream record",
        fields: {
          kind: { type: "text", value: "ai.chat.tool_result" },
          request_id: { type: "text", value: "req-1" },
          trace_id: { type: "text", value: "req-1" },
          tool_name: { type: "text", value: "ui.transactions.search" },
          result_hint: {
            type: "text",
            value: `matched ${txid} at ${address}`,
          },
        },
      },
      {
        id: "log-3",
        ts: "2026-05-17T18:00:02.000Z",
        level: "error",
        module: "daemon:bridge",
        file: "daemon/transport.ts",
        line: 0,
        msg: `failed with mnemonic=abandon and file ${localPath}`,
        fields: {
          kind: { type: "text", value: "ui.reports.tax_summary" },
          request_id: { type: "text", value: "req-1" },
          trace_id: { type: "text", value: "req-1" },
          error_code: { type: "text", value: "missing_price" },
          txid: { type: "txid", value: txid },
          address: { type: "address", value: address },
        },
      },
    ];

    const exported = exportSupportBundleRecords(records, {
      issueDescription: `Tax summary fails for ${txid} and ${url}`,
      header: {
        appVersion: "0.22.0 (abc1234)",
        os: "macOS",
        timeRange: "2026-05-17T18:00:00Z to 2026-05-17T18:01:00Z",
        activeFilter: "capture>=trace",
        redaction: "redacted-amounts",
        generatedAt: "2026-05-17T18:08:00Z",
      },
    });

    expect(exported).toContain("kassiber.support_bundle.manifest");
    expect(exported).toContain("kassiber.support_bundle.last_failure");
    expect(exported).toContain("kassiber.support_bundle.ai_provenance");
    expect(exported).toContain("missing_price");
    expect(exported).toContain("amount#");
    expect(exported).toContain("[redacted-url]");
    expect(exported).toContain("[redacted-txid]");
    expect(exported).toContain("[redacted-address]");
    expect(exported).not.toContain("1.234 BTC");
    expect(exported).not.toContain("xpub661MyMwAqRbcFsecret");
    expect(exported).not.toContain("sk-url-secret");
    expect(exported).not.toContain("dev@example.test");
    expect(exported).not.toContain(localPath);
    expect(exported).not.toContain(txid);
    expect(exported).not.toContain(address);
    expect(exported).not.toContain("mnemonic=abandon");
  });

  it("keeps logs in RAM and does not touch browser storage", () => {
    const setItem = vi.fn();
    vi.stubGlobal("localStorage", {
      getItem: vi.fn(),
      setItem,
      removeItem: vi.fn(),
      clear: vi.fn(),
    });

    emitAppLog(record({ detail: { type: "text", value: "first record" } }));
    const before = getAppLogBufferSize();
    expect(before).toBeGreaterThan(2);
    expect(setItem).not.toHaveBeenCalled();

    emitAppLog(record({ detail: { type: "text", value: "second record" } }));
    expect(getAppLogBufferSize()).toBeGreaterThan(before);
    expect(setItem).not.toHaveBeenCalled();
  });

  it("bounds the in-memory ring by count and approximate bytes", () => {
    for (let index = 0; index < APP_LOG_MAX_RECORDS + 25; index += 1) {
      emitAppLog(
        record({
          index: { type: "number", value: index },
          detail: { type: "text", value: "x".repeat(600) },
        }),
      );
    }

    expect(getAppLogRecords().length).toBeLessThanOrEqual(APP_LOG_MAX_RECORDS);
    expect(getAppLogBufferSize()).toBeLessThanOrEqual(APP_LOG_MAX_BYTES);
    expect(formatLogRecord(getAppLogRecords()[0], { redacted: true })).not.toContain(
      "index=0",
    );
  });
});
