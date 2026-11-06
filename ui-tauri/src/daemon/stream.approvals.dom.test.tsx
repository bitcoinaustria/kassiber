// @vitest-environment happy-dom
//
// Approvals a running turn waits on, keyed by call: each is answered on its
// own, leaves when the daemon runs or settles the call, and a permission mode
// switched to mid-turn reaches that turn and releases what it covers.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { DaemonStreamRecord } from "./transport";

const transport = vi.hoisted(() => ({
  stream: vi.fn(),
  invoke: vi.fn(),
}));

vi.mock("@/daemon/transport", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/daemon/transport")>()),
  getTransport: () => ({ stream: transport.stream, invoke: transport.invoke }),
}));

import { useAiChatStream, type AiChatRequest } from "./stream";

const REQUEST: AiChatRequest = {
  provider: "ollama",
  model: "qwen",
  messages: [{ role: "user", content: "Tidy my notes." }],
  permissionMode: "approval",
};

function hook() {
  const client = new QueryClient();
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return renderHook(() => useAiChatStream(), { wrapper });
}

/** Start a turn that stays open; returns a way to push records and finish. */
async function startTurn(result: ReturnType<typeof hook>["result"]) {
  let push!: (record: DaemonStreamRecord<unknown>) => void;
  let finish!: () => void;
  let requestId = "";
  transport.stream.mockImplementation(
    (request: { request_id: string }, options: { onRecord: typeof push }) => {
      requestId = request.request_id;
      push = (record) => options.onRecord({ request_id: requestId, ...record });
      return new Promise((resolve) => {
        finish = () =>
          resolve({ kind: "ai.chat", schema_version: 1, data: { finish_reason: "stop" } });
      });
    },
  );
  let sent!: Promise<unknown>;
  act(() => {
    sent = result.current.send(REQUEST, "Tidy my notes.");
  });
  return {
    requestId: () => requestId,
    push: (record: DaemonStreamRecord<unknown>) =>
      act(() => {
        push(record);
      }),
    finish: async () => {
      await act(async () => {
        finish();
        await sent;
      });
    },
  };
}

function consentRequired(callId: string, name = "ui.transactions.metadata_update") {
  return {
    kind: "ai.chat.tool_consent_required",
    schema_version: 1,
    data: { call_id: callId, name, summary: `Change ${callId}`, arguments_preview: {} },
  };
}

describe("pending approvals in the stream hook", () => {
  afterEach(() => {
    cleanup();
    transport.stream.mockReset();
    transport.invoke.mockReset();
  });

  it("answers each waiting call by its id", async () => {
    transport.invoke.mockResolvedValue({ kind: "ai.tool_call.consent", data: { recorded: true } });
    const { result } = hook();
    const turn = await startTurn(result);
    turn.push(consentRequired("c1"));
    turn.push(consentRequired("c2"));
    expect(result.current.pendingConsents.map((item) => item.callId)).toEqual(["c1", "c2"]);

    await act(async () => {
      await result.current.sendConsent("c2", "deny");
    });
    expect(transport.invoke).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "ai.tool_call.consent",
        args: { target_request_id: turn.requestId(), call_id: "c2", decision: "deny" },
      }),
    );
    expect(result.current.pendingConsents.map((item) => item.callId)).toEqual(["c1"]);

    // The daemon runs c1 (approved elsewhere, or covered by a mode): it leaves.
    turn.push({
      kind: "ai.chat.tool_call",
      schema_version: 1,
      data: { call_id: "c1", name: "ui.transactions.metadata_update", needs_consent: false, approval: "auto" },
    });
    expect(result.current.pendingConsents).toEqual([]);
    await turn.finish();
  });

  it("switches the running turn's mode and drops the approvals it released", async () => {
    transport.invoke.mockResolvedValue({
      kind: "ai.chat.permission_mode",
      data: { permission_mode: "full_access", released_call_ids: ["c1"] },
    });
    const { result } = hook();
    const turn = await startTurn(result);
    expect(transport.stream.mock.calls[0]![0]).toMatchObject({
      kind: "ai.chat",
      args: { permission_mode: "approval" },
    });
    turn.push(consentRequired("c1"));
    turn.push(consentRequired("c2"));

    let outcome: unknown;
    await act(async () => {
      outcome = await result.current.changePermissionMode("full_access");
    });
    expect(transport.invoke).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "ai.chat.permission_mode",
        args: { target_request_id: turn.requestId(), permission_mode: "full_access" },
      }),
    );
    expect(outcome).toEqual({ applied: true, releasedCallIds: ["c1"] });
    expect(result.current.pendingConsents.map((item) => item.callId)).toEqual(["c2"]);
    await turn.finish();
  });

  it("sends nothing when no turn runs, and reports a refusal", async () => {
    const { result } = hook();
    let idle: unknown;
    await act(async () => {
      idle = await result.current.changePermissionMode("auto");
    });
    expect(idle).toEqual({ applied: false, releasedCallIds: [] });
    expect(transport.invoke).not.toHaveBeenCalled();

    transport.invoke.mockResolvedValue({
      kind: "error",
      error: { code: "not_found", message: "No active chat" },
    });
    const turn = await startTurn(result);
    let refused: { applied: boolean; error?: { code: string } } | undefined;
    await act(async () => {
      refused = (await result.current.changePermissionMode("auto")) as typeof refused;
    });
    expect(refused?.applied).toBe(false);
    expect(refused?.error?.code).toBe("not_found");
    await turn.finish();
  });
});
