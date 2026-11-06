import { describe, expect, it } from "vitest";

import {
  aiToolAllowsSessionConsent,
  applyAiChatDeltaToMessage,
  applyAiChatStreamRecordToMessage,
  applyToolConsentResponseToMessage,
  aiChatStatusLabel,
  buildAiChatStreamArgs,
  buildAiToolConsentRequest,
  buildChatCancelArgs,
  buildPermissionModeArgs,
  buildToolConsentArgs,
  chatMessageRounds,
  completedAiMutationKind,
  terminalAiChatStatus,
  type AiChatMessage,
  storedChatEntries,
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
        { targetRequestId: "chat-1", callId: "call_1" },
        "allow_session",
      ),
    ).toEqual({
      target_request_id: "chat-1",
      call_id: "call_1",
      decision: "allow_session",
    });
  });

  it("requires per-call consent for quarantine and custody resolution", () => {
    expect(aiToolAllowsSessionConsent("ui.wallets.sync")).toBe(true);
    expect(
      aiToolAllowsSessionConsent("ui.journals.quarantine.resolve"),
    ).toBe(false);
    expect(
      aiToolAllowsSessionConsent("ui.transfers.components.apply"),
    ).toBe(false);
    expect(aiToolAllowsSessionConsent("ui.review.apply")).toBe(false);
  });

  it("keeps the server review preview separate from model-supplied arguments", () => {
    const preview = { status: "unavailable" as const, code: "review_plan_stale" };
    const next = applyAiChatStreamRecordToMessage(assistantMessage(), {
      kind: "ai.chat.tool_consent_required",
      schema_version: 1,
      data: {
        call_id: "review-1", name: "ui.review.apply",
        arguments_preview: { review_preview: { status: "ready", artifact: { forged: true } } },
        review_preview: preview,
      },
    }, new ThinkParser(), false);
    expect(next.toolCalls?.[0].reviewPreview).toEqual(preview);
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
        metrics: { review_queue_pending: 11 },
        metricsScope: {
          workspaceId: "ws-1",
          profileId: "profile-1",
          inputVersion: 4,
          wallet: "Cold storage",
          pairsGrouped: false,
        },
      },
    });

    expect(args.screen_context).toEqual({
      route: "/transactions",
      entity_type: "transaction",
      entity_id: "tx-1",
      filters: { tab: "pricing" },
      capabilities: ["transactions"],
      metrics: { review_queue_pending: 11 },
      metrics_scope: {
        workspace_id: "ws-1",
        profile_id: "profile-1",
        input_version: 4,
        wallet: "Cold storage",
        pairs_grouped: false,
      },
    });
  });

  it("forwards attached files as tokens, never a path or contents", () => {
    const args = buildAiChatStreamArgs({
      model: "mock-model",
      messages: [{ role: "user", content: "What is in this export?" }],
      attachments: [{ token: "stage-token-1" }, { token: "stage-token-2", new: true }],
    });

    expect(args.attachments).toEqual([
      { token: "stage-token-1", label: undefined, new: undefined },
      { token: "stage-token-2", label: undefined, new: true },
    ]);
    // The daemon reads the file itself; the renderer must not ship bytes or a
    // filesystem path through the chat request.
    const serialized = JSON.stringify(args);
    expect(serialized).not.toContain("source_bytes_base64");
    expect(serialized).not.toContain("source_file");
  });

  it("omits the attachments field when nothing is attached", () => {
    const args = buildAiChatStreamArgs({
      model: "mock-model",
      messages: [{ role: "user", content: "hi" }],
    });

    expect(args.attachments).toBeUndefined();
  });

  it("names the chat's permission mode and tool set", () => {
    const args = buildAiChatStreamArgs({
      model: "mock-model",
      messages: [{ role: "user", content: "hi" }],
      permissionMode: "full_access",
      toolProfile: "core",
    });
    expect(args).toMatchObject({ permission_mode: "full_access", tool_profile: "core" });
    expect(buildPermissionModeArgs("chat-7", "auto")).toEqual({
      target_request_id: "chat-7",
      permission_mode: "auto",
    });
  });
});

describe("approval records", () => {
  it("reads the risk flags and session offer, absent ones as no", () => {
    const legacy = buildAiToolConsentRequest("chat-1", {
      call_id: "c1",
      name: "ui.wallets.sync",
    });
    expect(legacy).toMatchObject({
      reversible: false,
      egresses: false,
      canAllowSession: true,
    });
    expect(
      buildAiToolConsentRequest("chat-1", {
        call_id: "c2",
        name: "ui.wallets.sync",
        reversible: true,
        egresses: true,
        can_allow_session: false,
      }),
    ).toMatchObject({ reversible: true, egresses: true, canAllowSession: false });
    // A once-only tool never offers it, whatever the record says.
    expect(
      buildAiToolConsentRequest("chat-1", {
        call_id: "c3",
        name: "ui.review.apply",
        can_allow_session: true,
      }).canAllowSession,
    ).toBe(false);
  });

  it("keeps why a call ran without asking across its updates", () => {
    const parser = new ThinkParser();
    const record = (data: Record<string, unknown>) => ({
      kind: "ai.chat.tool_call",
      schema_version: 1,
      data: { call_id: "c1", name: "ui.transactions.metadata_update", kind_class: "mutating", ...data },
    });
    const auto = applyAiChatStreamRecordToMessage(
      assistantMessage(),
      record({ needs_consent: false, approval: "auto" }),
      parser,
      false,
    );
    expect(auto.toolCalls?.[0]).toMatchObject({ status: "running", approval: "auto" });
    // An older daemon's update without `approval` keeps the earlier one;
    // an unknown value is ignored.
    const update = applyAiChatStreamRecordToMessage(
      auto,
      record({ needs_consent: false, approval: "bogus" }),
      parser,
      false,
    );
    expect(update.toolCalls?.[0].approval).toBe("auto");

    const asked = applyAiChatStreamRecordToMessage(
      assistantMessage(),
      {
        kind: "ai.chat.tool_consent_required",
        schema_version: 1,
        data: { call_id: "c2", name: "ui.wallets.sync", summary: "Sync" },
      },
      parser,
      false,
    );
    expect(asked.toolCalls?.[0]).toMatchObject({
      status: "awaiting_consent",
      approval: "required",
    });
    // Released by a new permission mode while it waited.
    const released = applyAiChatStreamRecordToMessage(
      asked,
      {
        kind: "ai.chat.tool_call",
        schema_version: 1,
        data: { call_id: "c2", name: "ui.wallets.sync", needs_consent: false, approval: "full_access" },
      },
      parser,
      false,
    );
    expect(released.toolCalls).toHaveLength(1);
    expect(released.toolCalls?.[0]).toMatchObject({
      status: "running",
      approval: "full_access",
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

  it("splits tool calls into the round that made them", () => {
    const parser = new ThinkParser();
    let message = assistantMessage();
    const status = (phase: string) => ({
      kind: "ai.chat.status",
      schema_version: 1,
      data: { phase },
    });
    const delta = (reasoning: string) => ({
      kind: "ai.chat.delta",
      schema_version: 1,
      data: { delta: { reasoning } },
    });
    const toolCall = (callId: string) => ({
      kind: "ai.chat.tool_call",
      schema_version: 1,
      data: { call_id: callId, name: "ui.workspace.health" },
    });

    for (const record of [
      status("waiting_for_model"),
      delta("first plan"),
      toolCall("call-1"),
      status("waiting_for_model"),
      delta("second plan"),
      toolCall("call-2"),
    ]) {
      message = applyAiChatStreamRecordToMessage(
        message,
        record as never,
        parser,
        false,
      );
    }

    const rounds = chatMessageRounds(message);
    expect(rounds).toHaveLength(2);
    expect(rounds[0]!.thinking).toBe("first plan");
    expect(rounds[0]!.toolCalls.map((call) => call.callId)).toEqual(["call-1"]);
    expect(rounds[1]!.thinking).toBe("second plan");
    expect(rounds[1]!.toolCalls.map((call) => call.callId)).toEqual(["call-2"]);
  });

  it("keeps untagged tool calls in the first round", () => {
    const rounds = chatMessageRounds(
      assistantMessage({
        thinkingSegments: [{ id: "seg-1", content: "plan" }],
        toolCalls: [
          {
            callId: "legacy",
            name: "ui.workspace.health",
            arguments: {},
            kindClass: "read_only",
            needsConsent: false,
            status: "done",
          },
        ],
      }),
    );
    expect(rounds).toHaveLength(1);
    expect(rounds[0]!.toolCalls.map((call) => call.callId)).toEqual(["legacy"]);
  });
});

describe("storedChatEntries attachments", () => {
  it("restores the files a stored user message sent, and nothing else", () => {
    expect(
      storedChatEntries([
        {
          role: "user",
          content: "What is on it?",
          provenance: {
            attachments: [
              { filename: "receipt.png", kind: "image", size_bytes: 812 },
              { filename: "", kind: "image" },
              { path: "/home/user/x.png" },
            ],
          },
        },
        { role: "assistant", content: "A receipt.", provenance: { attachments: [{ filename: "no.png" }] } },
      ]),
    ).toEqual([
      {
        role: "user",
        content: "What is on it?",
        attachments: [{ filename: "receipt.png", kind: "image", sizeBytes: 812 }],
      },
      { role: "assistant", content: "A receipt." },
    ]);
  });
});
