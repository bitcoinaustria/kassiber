import { describe, expect, it } from "vitest";

import {
  applyAiChatDeltaToMessage,
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
});
