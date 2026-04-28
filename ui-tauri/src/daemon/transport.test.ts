import { describe, expect, it } from "vitest";

import {
  readBridgeNdjsonStream,
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
});
