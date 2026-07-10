import { describe, expect, it } from "vitest";

import {
  applyAiChatDeltaToMessage,
  applyAiChatStreamRecordToMessage,
  applyToolConsentResponseToMessage,
  aiChatStatusLabel,
  buildAiChatStreamArgs,
  buildChatCancelArgs,
  buildToolConsentArgs,
  completedAiMutationKind,
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

  it("applies status records as visible loading activity", () => {
    const next = applyAiChatStreamRecordToMessage(
      assistantMessage(),
      {
        kind: "ai.chat.status",
        schema_version: 1,
        request_id: "chat-1",
        data: { phase: "waiting_for_model" },
      },
      new ThinkParser(),
      false,
    );

    expect(next.status).toBe("streaming");
    expect(next.activityLabel).toBe("Loading model");
    expect(aiChatStatusLabel({ phase: "thinking" })).toBe("Thinking");
  });

  it("marks reasoning-only deltas as thinking activity", () => {
    const next = applyAiChatDeltaToMessage(
      assistantMessage({ activityLabel: "Loading model" }),
      {
        kind: "ai.chat.delta",
        schema_version: 1,
        request_id: "chat-1",
        data: { delta: { reasoning: "Checking transactions..." } },
      },
      new ThinkParser(),
      false,
    );

    expect(next.content).toBe("");
    expect(next.thinking).toBe("Checking transactions...");
    expect(next.thinkingSegments).toEqual([
      { id: expect.any(String), content: "Checking transactions..." },
    ]);
    expect(next.activityLabel).toBe("Thinking");
  });

  it("splits reasoning into a new segment per waiting_for_model round", () => {
    const parser = new ThinkParser();
    let message = applyAiChatStreamRecordToMessage(
      assistantMessage(),
      {
        kind: "ai.chat.status",
        schema_version: 1,
        request_id: "chat-1",
        data: { phase: "waiting_for_model" },
      },
      parser,
      false,
    );
    message = applyAiChatDeltaToMessage(
      message,
      {
        kind: "ai.chat.delta",
        schema_version: 1,
        request_id: "chat-1",
        data: { delta: { reasoning: "Round one plan. " } },
      },
      parser,
      false,
    );
    message = applyAiChatStreamRecordToMessage(
      message,
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
    message = applyAiChatStreamRecordToMessage(
      message,
      {
        kind: "ai.chat.tool_result",
        schema_version: 1,
        request_id: "chat-1",
        data: {
          call_id: "call_1",
          ok: true,
          envelope: { kind: "ui.overview.snapshot", data: {} },
        },
      },
      parser,
      false,
    );
    message = applyAiChatStreamRecordToMessage(
      message,
      {
        kind: "ai.chat.status",
        schema_version: 1,
        request_id: "chat-1",
        data: { phase: "waiting_for_model" },
      },
      parser,
      false,
    );
    message = applyAiChatDeltaToMessage(
      message,
      {
        kind: "ai.chat.delta",
        schema_version: 1,
        request_id: "chat-1",
        data: { delta: { reasoning: "Round two after tools." } },
      },
      parser,
      false,
    );

    expect(message.thinkingSegments).toHaveLength(2);
    expect(message.thinkingSegments?.[0]?.content).toBe("Round one plan. ");
    expect(message.thinkingSegments?.[1]?.content).toBe(
      "Round two after tools.",
    );
    expect(message.thinking).toBe(
      "Round one plan. \n\nRound two after tools.",
    );
  });

  it("does not open a blank segment when waiting_for_model repeats", () => {
    const parser = new ThinkParser();
    let message = applyAiChatStreamRecordToMessage(
      assistantMessage(),
      {
        kind: "ai.chat.status",
        schema_version: 1,
        request_id: "chat-1",
        data: { phase: "waiting_for_model" },
      },
      parser,
      false,
    );
    message = applyAiChatStreamRecordToMessage(
      message,
      {
        kind: "ai.chat.status",
        schema_version: 1,
        request_id: "chat-1",
        data: { phase: "waiting_for_model" },
      },
      parser,
      false,
    );

    expect(message.thinkingSegments).toHaveLength(1);
    expect(message.thinkingSegments?.[0]?.content).toBe("");
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
          summary: "Refresh all watch-only sources",
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
          summary: "Refresh all watch-only sources",
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

describe("buildAiChatStreamArgs", () => {
  it("maps camelCase request fields onto the wire contract", () => {
    const args = buildAiChatStreamArgs({
      provider: "ollama",
      model: "mock-model",
      messages: [{ role: "user", content: "hello" }],
      toolsEnabled: true,
      toolLoopMaxIterations: 8,
      systemPromptKind: "kassiber",
      sessionId: "session-1",
      persist: "auto",
    });
    expect(args).toMatchObject({
      provider: "ollama",
      model: "mock-model",
      tools_enabled: true,
      tool_loop_max_iterations: 8,
      system_prompt_kind: "kassiber",
      session_id: "session-1",
      persist: "auto",
    });
  });

  it("omits the session id when absent and forwards incognito persist", () => {
    const args = buildAiChatStreamArgs({
      model: "mock-model",
      messages: [],
      persist: false,
    });
    expect(args.session_id).toBeUndefined();
    expect(args.persist).toBe(false);
  });

  it("forwards the typed ephemeral screen context", () => {
    const args = buildAiChatStreamArgs({
      model: "mock-model",
      messages: [{ role: "user", content: "Review this" }],
      screenContext: {
        route: "/transactions",
        entityType: "transaction",
        entityId: "tx-1",
        filters: { tab: "pricing" },
        capabilities: ["transactions"],
      },
    });

    expect(args.screen_context).toEqual({
      route: "/transactions",
      entity_type: "transaction",
      entity_id: "tx-1",
      filters: { tab: "pricing" },
      capabilities: ["transactions"],
    });
  });
});

describe("completedAiMutationKind", () => {
  it("returns a successful mutating tool once", () => {
    const observed = new Map();
    expect(
      completedAiMutationKind(observed, {
        kind: "ai.chat.tool_call",
        schema_version: 1,
        data: {
          call_id: "call-1",
          name: "ui.transactions.metadata.update",
          kind_class: "mutating",
        },
      }),
    ).toBeNull();
    expect(
      completedAiMutationKind(observed, {
        kind: "ai.chat.tool_result",
        schema_version: 1,
        data: { call_id: "call-1", ok: true },
      }),
    ).toBe("ui.transactions.metadata.update");
    expect(
      completedAiMutationKind(observed, {
        kind: "ai.chat.tool_result",
        schema_version: 1,
        data: { call_id: "call-1", ok: true },
      }),
    ).toBeNull();
  });

  it("ignores read-only and failed tool results", () => {
    const observed = new Map();
    completedAiMutationKind(observed, {
      kind: "ai.chat.tool_call",
      schema_version: 1,
      data: {
        call_id: "read-1",
        name: "ui.transactions.list",
        kind_class: "read_only",
      },
    });
    expect(
      completedAiMutationKind(observed, {
        kind: "ai.chat.tool_result",
        schema_version: 1,
        data: { call_id: "read-1", ok: true },
      }),
    ).toBeNull();

    completedAiMutationKind(observed, {
      kind: "ai.chat.tool_call",
      schema_version: 1,
      data: {
        call_id: "write-1",
        name: "ui.journals.process",
        kind_class: "mutating",
      },
    });
    expect(
      completedAiMutationKind(observed, {
        kind: "ai.chat.tool_result",
        schema_version: 1,
        data: { call_id: "write-1", ok: false, reason: "user_denied" },
      }),
    ).toBeNull();
  });
});
