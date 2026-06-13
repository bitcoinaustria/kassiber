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
  });

  it("always captures debug and trace records", () => {
    emitAppLog({ ...record(), level: "trace" });
    emitAppLog({ ...record(), level: "debug" });

    expect(getAppLogRecords()).toHaveLength(2);
    expect(getAppLogRecords().map((item) => item.level)).toEqual([
      "trace",
      "debug",
    ]);
  });

  it("applies the secret floor at insert so secrets never reach the ring", () => {
    const xpub =
      "xpub661MyMwAqRbcFtXgS5sYJABqqG9YLmC4Q1Rdap9gSE8NqtwybGhePY2gZ29ESFjqJoCu1Rupje8YtGqsefD265TMg7usUDFdp6W1EGMcet8";
    emitAppLog({
      ...record({
        detail: {
          type: "text",
          value: `electrum ssl://user:hunter2@electrum.example.test:50002 rejected descriptor=wpkh([abcd1234/84h/0h/0h]${xpub}/0/*)`,
        },
        duration_ms: { type: "duration_ms", value: 12 },
      }),
      msg: "wallet import failed: api_key=sk-local-secret",
    });

    const stored = getAppLogRecords()[0];
    expect(stored.msg).toBe("wallet import failed: api_key=[redacted]");
    const detail = String(stored.fields.detail.value);
    expect(detail).toContain("[redacted-credentials]@electrum.example.test");
    expect(detail).toContain("descriptor=[redacted]");
    expect(detail).not.toContain("hunter2");
    expect(detail).not.toContain(xpub);
    expect(stored.fields.duration_ms.value).toBe(12);
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
    expect(redacted).toContain("wallet_material=wpkh([abcd1234/84h/0h/0h][redacted-key])");
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
        activeFilter: "level=all, module=all, search=none",
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

  it("exports high-signal support bundles while enforcing the secret floor", () => {
    const txid = "a".repeat(64);
    const address = "bc1qexample000000000000000000000000000000f3xy";
    const xprv = "xprv9s21ZrQH143K2secretsecretsecretsecretsecret";
    const xpub = "xpub661MyMwAqRbcFsecretsecretsecretsecretsecret";
    const descriptor = `wpkh([abcd1234/84h/0h/0h]${xpub}/0/*)#deadbeef`;
    const mnemonic =
      "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    const url = "https://user:password@example.test/wallet?api_key=sk-url-secret";
    const localPath = "/Users/dev/.kassiber/data/kassiber.sqlite";
    const amountText = "0.12345678 BTC";
    const rateText = "BTC/EUR 64000.12";
    const records: AppLogRecord[] = [
      {
        ...record({
          request_id: { type: "text", value: "req-1" },
          trace_id: { type: "text", value: "req-1" },
          backend_url: { type: "url", value: url },
          email: { type: "email", value: "dev@example.test" },
          data_root: { type: "path", value: localPath },
          amount: { type: "amount", value: "1.234 BTC" },
          txid: { type: "txid", value: txid },
          address: { type: "address", value: address },
          wallet_xpub: { type: "xpub", value: xpub },
          wallet_xpriv: { type: "xpriv", value: xprv },
          wallet_descriptor: { type: "descriptor", value: descriptor },
          error_message: {
            type: "text",
            value: `Could not price ${txid} for ${address}`,
          },
          detail: {
            type: "text",
            value: `mnemonic=${mnemonic} raw_private=${xprv}`,
          },
        }),
        id: "log-1",
        ts: "2026-05-17T18:00:00.000Z",
        msg: `Could not price ${txid} for ${address} in ${localPath}`,
      },
    ];

    const exported = exportSupportBundleRecords(records, {
      issueDescription: `Tax summary lost ${amountText} at ${rateText} for ${txid} and ${url}`,
      header: {
        appVersion: "0.22.0 (abc1234)",
        os: "macOS",
        timeRange: "2026-05-17T18:00:00Z to 2026-05-17T18:01:00Z",
        activeFilter: "level=all",
        redaction: "high_signal",
        generatedAt: "2026-05-17T18:08:00Z",
      },
    });

    expect(exported).toContain('"redaction":"high_signal"');
    expect(exported).toContain("1.234 BTC");
    expect(exported).toContain(amountText);
    expect(exported).toContain(rateText);
    expect(exported).toContain(address);
    expect(exported).toContain(txid);
    expect(exported).toContain(localPath);
    expect(exported).toContain("dev@example.test");
    expect(exported).toContain("Could not price");
    expect(exported).toContain(`Could not price ${txid} for ${address} in ${localPath}`);
    expect(exported).toContain("https://[redacted-credentials]@example.test/wallet?api_key=[redacted]");
    expect(exported).toContain("xpub#");
    expect(exported).toContain("wpkh([abcd1234/84h/0h/0h][redacted-key])");
    expect(exported).toContain("[redacted-private-key]");
    expect(exported).not.toContain("sk-url-secret");
    expect(exported).not.toContain("user:password");
    expect(exported).not.toContain(xpub);
    expect(exported).not.toContain(xprv);
    expect(exported).not.toContain("deadbeef");
    for (const word of mnemonic.split(" ")) {
      expect(exported).not.toContain(word);
    }
  });

  it("exports public-safe support bundles with failure context and AI provenance", () => {
    const txid = "a".repeat(64);
    const address = "bc1qexample000000000000000000000000000000f3xy";
    const legacyAddress = "1BoatSLRHtKNngkdXEeobR76b53LETtpyT";
    const scriptHashAddress = "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy";
    const liquidAddress = "lq1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";
    const descriptor = "wpkh([abcd1234/84h/0h/0h]xpub661MyMwAqRbcFsecret/0/*)";
    const url = "https://user:secret@example.test/wallet?api_key=sk-url-secret";
    const localPath = "/Users/dev/.kassiber/data/kassiber.sqlite";
    const amountText = "0.12345678 BTC";
    const fiatAmountText = "\u20ac12,345.67";
    const prefixedFiatAmountText = "USD 42.10";
    const satsText = "2500 sats";
    const rateText = "BTC/EUR 64000.12";
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
          error_message: {
            type: "text",
            value: `lost ${amountText}, fee ${satsText}, fiat ${fiatAmountText}, proceeds ${prefixedFiatAmountText}, rate ${rateText}`,
          },
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
            value: `matched ${txid} at ${address} ${legacyAddress} ${scriptHashAddress} ${liquidAddress}`,
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
      issueDescription: `Tax summary lost ${amountText}, fiat ${fiatAmountText}, proceeds ${prefixedFiatAmountText}, rate ${rateText}, and ${url}`,
      header: {
        appVersion: "0.22.0 (abc1234)",
        os: "macOS",
        timeRange: "2026-05-17T18:00:00Z to 2026-05-17T18:01:00Z",
        activeFilter: "level=all",
        redaction: "redacted-amounts",
        generatedAt: "2026-05-17T18:08:00Z",
      },
      mode: "public_safe",
    });

    expect(exported).toContain("kassiber.support_bundle.manifest");
    expect(exported).toContain("kassiber.support_bundle.last_failure");
    expect(exported).toContain("kassiber.support_bundle.ai_provenance");
    expect(exported).toContain('"redaction":"public_safe"');
    expect(exported).toContain("missing_price");
    expect(exported).toContain("amount#");
    expect(exported).toContain("[redacted-amount]");
    expect(exported).toContain("[redacted-rate]");
    expect(exported).toContain("[redacted-url]");
    expect(exported).toContain("[redacted-txid]");
    expect(exported).toContain("[redacted-address]");
    expect(exported).not.toContain("1.234 BTC");
    expect(exported).not.toContain(amountText);
    expect(exported).not.toContain(fiatAmountText);
    expect(exported).not.toContain(prefixedFiatAmountText);
    expect(exported).not.toContain(satsText);
    expect(exported).not.toContain(rateText);
    expect(exported).not.toContain("xpub661MyMwAqRbcFsecret");
    expect(exported).not.toContain("sk-url-secret");
    expect(exported).not.toContain("dev@example.test");
    expect(exported).not.toContain(localPath);
    expect(exported).not.toContain(txid);
    expect(exported).not.toContain(address);
    expect(exported).not.toContain(legacyAddress);
    expect(exported).not.toContain(scriptHashAddress);
    expect(exported).not.toContain(liquidAddress);
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
