import { describe, expect, it } from "vitest";

import {
  applyAiChatDeltaToMessage,
  applyAiChatStreamRecordToMessage,
  terminalAiChatStatus,
  type AiChatMessage,
} from "./stream";
import { ThinkParser } from "@/lib/thinkParser";

function assistantMessage(overrides: Partial<AiChatMessage> = {}): AiChatMessage {
  return {
    id: "assistant-1",
    role: "assistant",
    content: "",
    thinking: "",
    status: "pending",
    ...overrides,
  };
}

describe("AI stream reducer helpers", () => {
  it("ignores late deltas after abort", () => {
    const current = assistantMessage({
      content: "already stopped",
      status: "cancelled",
    });

    const next = applyAiChatDeltaToMessage(
      current,
      {
        kind: "ai.chat.delta",
        schema_version: 1,
        request_id: "chat-1",
        data: { delta: { content: " late token" } },
      },
      new ThinkParser(),
      true,
    );

    expect(next).toBe(current);
    expect(next.content).toBe("already stopped");
    expect(next.status).toBe("cancelled");
  });

  it("maps terminal cancelled finish reason to cancelled state", () => {
    expect(terminalAiChatStatus("cancelled", false)).toBe("cancelled");
    expect(terminalAiChatStatus("stop", true)).toBe("cancelled");
    expect(terminalAiChatStatus("stop", false)).toBe("done");
  });

  it("applies tool call, tool result, and final delta records", () => {
    const parser = new ThinkParser();
    const withCall = applyAiChatStreamRecordToMessage(
      assistantMessage(),
      {
        kind: "ai.chat.tool_call",
        schema_version: 1,
        request_id: "chat-1",
        data: {
          call_id: "call_1",
          name: "ui.overview.snapshot",
          arguments: {},
          kind_class: "read_only",
          needs_consent: false,
        },
      },
      parser,
      false,
    );

    expect(withCall.toolCalls).toHaveLength(1);
    expect(withCall.toolCalls?.[0].status).toBe("running");

    const withResult = applyAiChatStreamRecordToMessage(
      withCall,
      {
        kind: "ai.chat.tool_result",
        schema_version: 1,
        request_id: "chat-1",
        data: {
          call_id: "call_1",
          ok: true,
          envelope: { kind: "ui.overview.snapshot", data: { txs: [] } },
        },
      },
      parser,
      false,
    );

    expect(withResult.toolCalls?.[0].status).toBe("done");

    const withDelta = applyAiChatStreamRecordToMessage(
      withResult,
      {
        kind: "ai.chat.delta",
        schema_version: 1,
        request_id: "chat-1",
        data: { delta: { content: "Ready." } },
      },
      parser,
      false,
    );

    expect(withDelta.content).toBe("Ready.");
    expect(withDelta.toolCalls?.[0].status).toBe("done");
  });
});
