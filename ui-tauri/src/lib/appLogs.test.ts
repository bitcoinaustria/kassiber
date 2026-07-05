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

  it("floors a secret hiding in a non-secret typed field at insert", () => {
    // Declared secret types are masked at render, but a credential can ride in
    // under an operational/free type too (here a `url` with an api_key). The
    // insert floor must scrub it so it never sits raw in the ring or a
    // high-signal export, not only in a `text` field.
    emitAppLog(
      record({
        endpoint: {
          type: "url",
          value: "https://api.example.test/v1?api_key=sk-leaked-001",
        },
      }),
    );
    const endpoint = String(getAppLogRecords()[0].fields.endpoint.value);
    expect(endpoint).not.toContain("sk-leaked-001");
    expect(endpoint).toContain("[redacted]");
  });

  it("redacts JSON-shaped secrets in logged objects at insert", () => {
    // console.error(obj) JSON-stringifies its argument, so the secret floor
    // must catch quoted "key":"value" assignments, not just key=value text.
    emitAppLog({
      ...record(),
      module: "console",
      msg: 'request body {"api_key":"sk-json-secret","passphrase": "hunter2","note":"keep"}',
    });

    const stored = getAppLogRecords()[0];
    expect(stored.msg).not.toContain("sk-json-secret");
    expect(stored.msg).not.toContain("hunter2");
    expect(stored.msg).toContain('"api_key":"[redacted]"');
    expect(stored.msg).toContain('"note":"keep"');
  });

  it("applies the secret floor to Silent Payments material", () => {
    const spscan = `spscan1q${"p".repeat(120)}`;
    const spspend = `spspend1q${"q".repeat(120)}`;
    const spAddress = `sp1q${"p".repeat(120)}`;
    const descriptor = `sp(${spscan})`;

    emitAppLog({
      ...record({
        detail: {
          type: "text",
          value: `free text ${descriptor} ${spspend} ${spAddress}`,
        },
      }),
      msg: `silent_payment_material=${descriptor} {"sp_descriptor":"${descriptor}"}`,
    });

    const stored = getAppLogRecords()[0];
    const combined = `${stored.msg} ${stored.fields.detail.value}`;
    expect(combined).toContain("[redacted]");
    expect(combined).toContain("sp([redacted-key])");
    expect(combined).toContain("[redacted-silent-payment-key]");
    expect(combined).toContain("[redacted-silent-payment-address]");
    expect(combined).not.toContain(spscan);
    expect(combined).not.toContain(spspend);
    expect(combined).not.toContain(spAddress);
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
        data_root: {
          type: "path",
          value: "/Users/dev/.kassiber/projects/family/data",
        },
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
    // amounts are always pseudonymized; with scale shown they keep a coarse magnitude
    expect(redacted).toContain("amount=amount#");
    expect(redacted).toContain("(~1 BTC)");
    expect(redacted).not.toContain("1.234 BTC");
    expect(redacted).not.toContain("xpub");
    expect(redacted).not.toContain("descriptor");
    expect(redacted).not.toContain("/Users/");
    expect(redacted).not.toContain("Treasury hot wallet");

    expect(stableMaskedValue({ type: "label", value: "Treasury hot wallet" })).toBe(
      stableMaskedValue({ type: "label", value: "Treasury hot wallet" }),
    );
    expect(stableMaskedValue({ type: "amount", value: "2500 sats" })).not.toBe(
      "amount#c2d60ca1",
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
    const localPath = "/Users/dev/.kassiber/projects/family/data/kassiber.sqlite";
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
    // txids + amounts are pseudonymized even in high_signal (the AI-debug tier),
    // never raw — they are the wallet fingerprint.
    expect(exported).not.toContain(txid);
    expect(exported).not.toContain("1.234 BTC");
    expect(exported).not.toContain(amountText);
    expect(exported).toContain("txid#");
    expect(exported).toContain("amount#");
    expect(exported).toContain("(~1 BTC)"); // typed amount keeps a coarse magnitude
    expect(exported).toContain("(~0.1 BTC)"); // free-text amount in the issue description
    // the same txid collapses to ONE stable token everywhere it appears, so
    // cross-line correlation (the real debugging signal) survives.
    const txidTokens = new Set(
      [...exported.matchAll(/txid#[0-9a-f]{8}/g)].map((m) => m[0]),
    );
    expect(txidTokens.size).toBe(1);
    // operational data the owner chose to keep readable in high_signal
    expect(exported).toContain(rateText);
    expect(exported).toContain(address);
    expect(exported).toContain(localPath);
    expect(exported).toContain("dev@example.test");
    expect(exported).toContain("Could not price txid#");
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
    const localPath = "/Users/dev/.kassiber/projects/family/data/kassiber.sqlite";
    const amountText = "0.12345678 BTC";
    const fiatAmountText = "\u20ac12,345.67";
    const prefixedFiatAmountText = "USD 42.10";
    const satsText = "2500 sats";
    const signedSatsText = "-2500 sats";
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
            value: `lost ${amountText}, fee ${satsText}, signed fee ${signedSatsText}, fiat ${fiatAmountText}, proceeds ${prefixedFiatAmountText}, rate ${rateText}`,
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
    // txids + amounts are stable pseudonyms in public_safe too, but with NO
    // magnitude (the public tier drops scale).
    expect(exported).toContain("amount#");
    expect(exported).toContain("txid#");
    expect(exported).not.toContain("[redacted-amount]");
    expect(exported).not.toContain("[redacted-txid]");
    expect(exported).not.toContain("(~");
    expect(exported).toContain("[redacted-rate]");
    expect(exported).toContain("[redacted-url]");
    expect(exported).toContain("[redacted-address]");
    expect(exported).not.toContain("1.234 BTC");
    expect(exported).not.toContain(amountText);
    expect(exported).not.toContain(fiatAmountText);
    expect(exported).not.toContain(prefixedFiatAmountText);
    expect(exported).not.toContain(satsText);
    expect(exported).not.toContain(signedSatsText);
    expect(exported).not.toContain("amount#c2d60ca1");
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

  it("pseudonymizes txids and amounts in both tiers while keeping them correlatable", () => {
    const txid = "b".repeat(64);
    const otherTxid = "c".repeat(64);
    const emitted = emitAppLog({
      ...record({
        txid: { type: "txid", value: txid },
        fee: { type: "amount", value: "2500 sats" },
      }),
      msg: `synced ${txid} fee 0.0001 BTC, signed fee -2500 sats, rate BTC/EUR 64000.12, dash rate BTC-EUR 64000.12, also ${otherTxid}`,
    });
    expect(emitted).not.toBeNull();

    // cross-impl contract: the daemon mirror (kassiber/redaction.py::_stable_hash)
    // pins this same literal so a txid pseudonymized on either side is one token.
    expect(stableMaskedValue({ type: "txid", value: "a".repeat(64) })).toBe(
      "txid#d96f0f85",
    );

    // high_signal: txids/amounts pseudonymized (never raw), amounts keep a
    // coarse magnitude, market rate stays readable.
    const high = formatLogRecord(emitted!, { redacted: true, mode: "high_signal" });
    expect(high).not.toContain(txid);
    expect(high).not.toContain(otherTxid);
    expect(high).toContain("txid#");
    expect(high).toContain("amount#");
    expect(high).toContain("(~0.0001 BTC)"); // free-text amount keeps magnitude
    expect(high).toContain("(~-1000 sats)"); // signed amount keeps only coarse magnitude
    expect(high).toContain("BTC/EUR 64000.12"); // market rate is public, stays readable
    expect(high).toContain("BTC-EUR 64000.12");
    expect(high).not.toContain("-2500 sats");
    expect(high).not.toContain("amount#c2d60ca1");

    // The txid in the typed field and in the message resolve to the SAME token
    // (cross-line correlation), and the two distinct txids get two tokens.
    const fieldToken = high.match(/txid=(txid#[0-9a-f]{8})/)?.[1];
    expect(fieldToken).toBeTruthy();
    expect(high.split(fieldToken!).length - 1).toBeGreaterThanOrEqual(2);
    const tokens = new Set([...high.matchAll(/txid#[0-9a-f]{8}/g)].map((m) => m[0]));
    expect(tokens.size).toBe(2);

    // public_safe with scale hidden: same correlatable token, no magnitude.
    const pub = formatLogRecord(emitted!, {
      redacted: true,
      mode: "public_safe",
      maskAmounts: true,
    });
    expect(pub).toContain("amount#");
    expect(pub).toContain(fieldToken!); // same pseudonym across tiers
    expect(pub).not.toContain("(~");
    expect(pub).not.toContain(txid);
    expect(pub).not.toContain("-2500 sats");
    expect(pub).not.toContain("amount#c2d60ca1");
  });

  it("pseudonymizes keyed sat amounts, long hex runs, and the search-query header", () => {
    const txid = "d".repeat(64);
    const longHex = "e".repeat(72);
    const emitted = emitAppLog({
      ...record(),
      msg: `fee_msat=100000 and amount_sat=50000 for ${txid} blob ${longHex}`,
    });
    const high = formatLogRecord(emitted!, { redacted: true, mode: "high_signal" });
    // keyed/glued sat amounts: the integer is gone, the key stays readable
    expect(high).toContain("fee_msat=amount#");
    expect(high).toContain("amount_sat=amount#");
    expect(high).not.toContain("=100000");
    expect(high).not.toContain("=50000");
    // a >64-hex run is pseudonymized as one token, not left raw past 64 chars
    expect(high).not.toContain(longHex);
    expect(high).not.toContain(txid);
    expect(high).toContain("txid#");

    // the md export header's active filter carries the user's raw search query
    // (here a pasted txid) and must be scrubbed in a redacted export
    const md = exportLogRecords([emitted!], "md", {
      redacted: true,
      mode: "high_signal",
      header: {
        appVersion: "0.0.0",
        os: "macOS",
        timeRange: "all",
        activeFilter: `search=${txid}`,
        redaction: "high_signal",
        generatedAt: "2026-06-23T00:00:00Z",
      },
    });
    expect(md).toContain("Active filter:");
    expect(md).not.toContain(txid);
    expect(md).toContain("txid#");
  });

  it("hashes a typed currency-symbol amount the same as its free-text form", () => {
    // parseTypedAmount must fold a leading currency symbol into the unit so the
    // typed-field and free-text paths produce the same amount# token.
    const typed = emitAppLog({
      ...record({ fee: { type: "amount", value: "€12,345.67" } }),
      msg: "typed",
    });
    const typedOut = formatLogRecord(typed!, { redacted: true, mode: "high_signal" });
    const typedToken = typedOut.match(/fee=(amount#[0-9a-f]{8})/)?.[1];

    const free = emitAppLog({ ...record(), msg: "charged €12,345.67 today" });
    const freeOut = formatLogRecord(free!, { redacted: true, mode: "high_signal" });
    const freeToken = freeOut.match(/(amount#[0-9a-f]{8})/)?.[1];

    expect(typedToken).toBeTruthy();
    expect(typedToken).toBe(freeToken);
    expect(typedOut).not.toContain("12,345.67");
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
