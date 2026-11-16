import { describe, expect, it } from "vitest";

import {
  canResetRegtestDemo,
  envelopeLogLevel,
  isRegtestDemoDataRoot,
  normalizeExternalBrowserUrl,
  readBridgeNdjsonStream,
  regtestDemoDataRoot,
  summarizeDaemonCompletionFields,
  summarizeEnvelopeFields,
  type DaemonStreamRecord,
} from "./transport";

function ndjsonResponse(chunks: string[]): Response {
  const encoder = new TextEncoder();
  return new Response(
    new ReadableStream({
      start(controller) {
        for (const chunk of chunks) {
          controller.enqueue(encoder.encode(chunk));
        }
        controller.close();
      },
    }),
    { status: 200 },
  );
}

describe("bridge NDJSON stream reader", () => {
  it("routes intermediate records and returns the terminal envelope", async () => {
    const records: DaemonStreamRecord[] = [];
    const terminal = await readBridgeNdjsonStream(
      ndjsonResponse([
        '{"kind":"ai.chat.delta","schema_version":1,"request_id":"chat-1","data":{"delta":{"content":"Hel',
        'lo"}}}\n{"kind":"ai.chat","schema_version":1,"request_id":"chat-1","data":{"finish_reason":"stop"}}\n',
      ]),
      "ai.chat",
      "chat-1",
      { onRecord: (record) => records.push(record) },
    );

    expect(records).toHaveLength(1);
    expect(records[0].kind).toBe("ai.chat.delta");
    expect(terminal.kind).toBe("ai.chat");
    expect(terminal.data).toEqual({ finish_reason: "stop" });
  });

  it("suppresses intermediate records after abort but keeps terminal resolution", async () => {
    const controller = new AbortController();
    controller.abort();
    const records: DaemonStreamRecord[] = [];
    const terminal = await readBridgeNdjsonStream(
      ndjsonResponse([
        '{"kind":"ai.chat.delta","schema_version":1,"request_id":"chat-1","data":{"delta":{"content":"late"}}}\n',
        '{"kind":"ai.chat","schema_version":1,"request_id":"chat-1","data":{"finish_reason":"cancelled"}}\n',
      ]),
      "ai.chat",
      "chat-1",
      {
        signal: controller.signal,
        onRecord: (record) => records.push(record),
      },
    );

    expect(records).toEqual([]);
    expect(terminal.data).toEqual({ finish_reason: "cancelled" });
  });

  it("treats auth_required as a terminal envelope", async () => {
    const records: DaemonStreamRecord[] = [];
    const terminal = await readBridgeNdjsonStream(
      ndjsonResponse([
        '{"kind":"auth_required","schema_version":1,"request_id":"chat-locked","data":{"scope":"unlock_database"}}\n',
      ]),
      "ai.chat",
      "chat-locked",
      { onRecord: (record) => records.push(record) },
    );

    expect(records).toEqual([]);
    expect(terminal.kind).toBe("auth_required");
    expect(terminal.data).toEqual({ scope: "unlock_database" });
  });
});

describe("daemon envelope logging summaries", () => {
  it("keeps the original request kind when the terminal envelope is an error", () => {
    const summary = summarizeDaemonCompletionFields(
      { kind: "ui.journals.process", request_id: "journal-1" },
      {
        kind: "error",
        schema_version: 1,
        request_id: "journal-1",
        error: {
          code: "database_busy",
          message: "The local database is busy.",
          retryable: true,
        },
      },
      3570.4,
    );

    expect(summary.kind).toEqual({ type: "text", value: "error" });
    expect(summary.request_kind).toEqual({
      type: "text",
      value: "ui.journals.process",
    });
    expect(summary.duration_ms).toEqual({ type: "duration_ms", value: 3570 });
  });

  it("marks failed wallet sync result rows as error-level with useful context", () => {
    const summary = summarizeEnvelopeFields({
      kind: "ui.wallets.sync",
      schema_version: 1,
      request_id: "sync-1",
      data: {
        ok: false,
        results: [
          {
            wallet: "Satoshi-Liquid",
            status: "error",
            code: "backend_sync_failed",
            message:
              "Source refresh failed for Satoshi-Liquid during backend_fetch: invalid literal for int() with base 10: '2026-04-14T10:17:10Z'",
            hint: "Test the selected sync backend in Settings, then retry refresh.",
            retryable: true,
            details: {
              backend: "liquid",
              backend_kind: "electrum",
              chain: "liquid",
              network: "liquidv1",
              phase: "backend_fetch",
              error_type: "ValueError",
              has_backend_url: true,
            },
          },
        ],
      },
    });

    expect(
      envelopeLogLevel({
        kind: "ui.wallets.sync",
        schema_version: 1,
        data: { results: [{ status: "error" }] },
      }),
    ).toBe("error");
    expect(summary.sync_error_wallet).toEqual({
      type: "label",
      value: "Satoshi-Liquid",
    });
    expect(summary.sync_error_code).toEqual({
      type: "text",
      value: "backend_sync_failed",
    });
    expect(summary.sync_error_phase).toEqual({
      type: "text",
      value: "backend_fetch",
    });
    expect(summary.sync_error_backend_kind).toEqual({
      type: "text",
      value: "electrum",
    });
    expect(summary.sync_error_has_backend_url).toEqual({
      type: "boolean",
      value: true,
    });
  });
});

describe("external URL opener validation", () => {
  it("normalizes HTTP and HTTPS browser URLs", () => {
    expect(
      normalizeExternalBrowserUrl(" https://mempool.space/tx/abc123 "),
    ).toBe("https://mempool.space/tx/abc123");
    expect(
      normalizeExternalBrowserUrl("http://127.0.0.1:3002/tx/abc123"),
    ).toBe("http://127.0.0.1:3002/tx/abc123");
  });

  it("rejects non-browser URLs", () => {
    for (const url of [
      "",
      "/tx/abc123",
      "file:///tmp/report.pdf",
      "ftp://example.test/tx/abc123",
      "javascript:alert(1)",
      "mailto:dev@example.test",
    ]) {
      expect(() => normalizeExternalBrowserUrl(url), url).toThrow();
    }
  });

  it("rejects URLs with embedded credentials", () => {
    for (const url of [
      "https://dev@example.test/tx/abc123",
      "https://dev:secret@example.test/tx/abc123",
    ]) {
      expect(() => normalizeExternalBrowserUrl(url), url).toThrow();
    }
  });
});

describe("regtest demo reset bridge gating", () => {
  it("only enables reset for the configured demo data root", () => {
    const demoRoot = regtestDemoDataRoot();

    expect(demoRoot).toContain("regtest-demo");
    expect(canResetRegtestDemo(demoRoot)).toBe(true);
    expect(canResetRegtestDemo(`${demoRoot}/`)).toBe(true);
    expect(isRegtestDemoDataRoot(`${demoRoot}///`)).toBe(true);
    expect(canResetRegtestDemo("/tmp/other-regtest-book/data")).toBe(false);
  });
});

describe("exchange auth errors in the RAM log and support exports", () => {
  it("carry only Kassiber's own code values through both export tiers", async () => {
    const { clearAppLogRecords, emitAppLog, exportSupportBundleRecords, getAppLogRecords } =
      await import("@/lib/appLogs");
    const key = "synthetic-exchange-key";
    // The error the daemon sends when Bitpanda's 401 echoes the request's
    // API key as its error code: the code is mapped to "unknown" in Rust
    // (exchange_api tests: provider_codes_are_local_values_never_the_providers_string).
    const envelope = {
      kind: "error",
      schema_version: 1,
      request_id: "req-exchange",
      error: {
        code: "auth_error",
        message: "Bitpanda refused the API key (HTTP 401)",
        hint: "Check that the key is current, allowed from this IP address, and has the Transaction and Trade (Read) scopes.",
        details: { http_status: 401, bitpanda_error: "unknown" },
        retryable: false,
      },
    };
    clearAppLogRecords();
    emitAppLog({
      level: "warning",
      module: "daemon:bridge",
      file: "daemon/transport.ts",
      line: 0,
      msg: "Daemon invoke failed",
      fields: summarizeEnvelopeFields(envelope),
    });
    for (const mode of ["high_signal", "public_safe"] as const) {
      const exported = exportSupportBundleRecords(getAppLogRecords(), {
        issueDescription: "exchange sync failed",
        header: {
          appVersion: "0.22.0 (abc1234)",
          os: "Linux",
          timeRange: "2026-11-14T00:00:00Z to 2026-11-14T00:01:00Z",
          activeFilter: "level=all",
          redaction: "redacted-amounts",
          generatedAt: "2026-11-14T00:02:00Z",
        },
        mode,
      });
      expect(exported).toContain("bitpanda_error");
      expect(exported).toContain("unknown");
      expect(exported).not.toContain(key);
    }
  });
});
