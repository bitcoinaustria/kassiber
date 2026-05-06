import { describe, expect, it } from "vitest";

import {
  normalizeExternalBrowserUrl,
  readBridgeNdjsonStream,
  redactForLog,
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

describe("daemon log redaction", () => {
  it("redacts structured secret fields recursively", () => {
    expect(
      redactForLog({
        limit: 25,
        auth_response: { passphrase_secret: "correct horse battery staple" },
        nested: { api_key: "sk-local", backendToken: "btcpay-token" },
      }),
    ).toEqual({
      limit: 25,
      auth_response: "[redacted]",
      nested: { api_key: "[redacted]", backendToken: "[redacted]" },
    });
  });

  it("redacts secret-looking strings in stderr-style details", () => {
    expect(
      redactForLog(
        "token=btcpay-token Bearer abc.def.ghi wpkh([abcd1234/84h/0h/0h]xpub661MyMwAqRbcF12345678901234567890/0/*)",
      ),
    ).toBe(
      "token=[redacted] Bearer [redacted] [redacted-descriptor]",
    );
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
