import { afterEach, describe, expect, it, vi } from "vitest";

import {
  getTransport,
  normalizeExternalBrowserUrl,
  readBridgeNdjsonStream,
  type DaemonStreamRecord,
} from "./transport";

const originalFetch = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
});

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

describe("bridge daemon invoke transport", () => {
  it("allocates a request id for regular invokes", async () => {
    const requests: unknown[] = [];
    globalThis.fetch = vi.fn(async (_input, init) => {
      const body = JSON.parse(String(init?.body ?? "{}"));
      requests.push(body);
      return new Response(
        JSON.stringify({
          kind: "status",
          schema_version: 1,
          request_id: body.request_id,
          data: {},
        }),
      );
    }) as typeof fetch;

    const envelope = await getTransport("real").invoke({ kind: "status" });
    const request = requests[0] as { request_id?: unknown };

    expect(typeof request.request_id).toBe("string");
    expect(String(request.request_id)).not.toHaveLength(0);
    expect(envelope.request_id).toBe(request.request_id);
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
