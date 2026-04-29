import { describe, expect, it } from "vitest";

import {
  applyAiChatDeltaToMessage,
  applyAiChatStreamRecordToMessage,
  applyToolConsentResponseToMessage,
  buildChatCancelArgs,
  buildToolConsentArgs,
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

  it("applies consent required, denial, and consent request args", () => {
    const parser = new ThinkParser();
    const withPrompt = applyAiChatStreamRecordToMessage(
      assistantMessage(),
      {
        kind: "ai.chat.tool_consent_required",
        schema_version: 1,
        request_id: "chat-1",
        data: {
          call_id: "call_1",
          name: "ui.wallets.sync",
          summary: "Sync all wallets",
          arguments_preview: { descriptor: "<redacted>" },
        },
      },
      parser,
      false,
    );

    expect(withPrompt.toolCalls).toHaveLength(1);
    expect(withPrompt.toolCalls?.[0].status).toBe("awaiting_consent");
    expect(withPrompt.toolCalls?.[0].arguments).toEqual({
      descriptor: "<redacted>",
    });

    const withDenied = applyAiChatStreamRecordToMessage(
      withPrompt,
      {
        kind: "ai.chat.tool_result",
        schema_version: 1,
        request_id: "chat-1",
        data: {
          call_id: "call_1",
          ok: false,
          reason: "user_denied",
        },
      },
      parser,
      false,
    );

    expect(withDenied.toolCalls?.[0].status).toBe("denied");
    expect(withDenied.toolCalls?.[0].reason).toBe("user_denied");
    expect(
      buildToolConsentArgs(
        {
          targetRequestId: "chat-1",
          callId: "call_1",
          name: "ui.wallets.sync",
          summary: "Sync all wallets",
          argumentsPreview: {},
        },
        "allow_session",
      ),
    ).toEqual({
      target_request_id: "chat-1",
      call_id: "call_1",
      decision: "allow_session",
    });
  });

  it("builds a targeted daemon cancel request for active chats", () => {
    expect(buildChatCancelArgs("chat-active-1")).toEqual({
      target_request_id: "chat-active-1",
    });
  });

  it("marks stale consent acknowledgements as tool errors", () => {
    const current = assistantMessage({
      toolCalls: [
        {
          callId: "call_1",
          name: "ui.wallets.sync",
          arguments: {},
          kindClass: "mutating",
          needsConsent: true,
          status: "awaiting_consent",
        },
      ],
    });

    const stale = applyToolConsentResponseToMessage(
      current,
      "call_1",
      "allow_once",
      false,
      "not_found",
    );

    expect(stale.toolCalls?.[0].status).toBe("error");
    expect(stale.toolCalls?.[0].reason).toBe("not_found");

    const recordedDeny = applyToolConsentResponseToMessage(
      current,
      "call_1",
      "deny",
      true,
    );

    expect(recordedDeny.toolCalls?.[0].status).toBe("denied");
    expect(recordedDeny.toolCalls?.[0].reason).toBe("user_denied");
  });
});
