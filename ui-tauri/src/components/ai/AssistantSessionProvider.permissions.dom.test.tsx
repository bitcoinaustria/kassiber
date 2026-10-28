// @vitest-environment happy-dom
//
// Every turn names the chat's permission mode and tool set. A new chat starts
// in the Settings default mode and its provider's tool set (all tools unless
// the user picked fewer); the composer changes both for this chat, and a mode
// switched to while an answer streams reaches that answer.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const transport = vi.hoisted(() => ({
  stream: vi.fn(),
  invoke: vi.fn(),
}));

vi.hoisted(() => {
  const makeStorage = () => {
    const storage = new Map<string, string>();
    return {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => void storage.set(key, value),
      removeItem: (key: string) => void storage.delete(key),
    };
  };
  vi.stubGlobal("localStorage", makeStorage());
  vi.stubGlobal("sessionStorage", makeStorage());
});

vi.mock("@/daemon/transport", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/daemon/transport")>()),
  getTransport: () => ({ stream: transport.stream, invoke: transport.invoke }),
}));
vi.mock("@/components/ai/useActiveBookScope", () => ({
  useActiveBookScope: () => ({ workspace_id: "ws-1", profile_id: "profile-1" }),
}));
vi.mock("@/components/ai/useProviderDefaultEffort", () => ({
  useProviderDefaultEffort: ({
    setThinkingEffort,
  }: {
    setThinkingEffort: (effort: string) => void;
  }) => ({ setThinkingEffort, effortSeeded: true }),
}));
vi.mock("@/components/ai/useFastModeSupport", () => ({
  useFastModeSupport: () => ({ supported: false, resolved: true, runtime: null }),
}));

import {
  useAssistantSession,
  type AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import { useUiStore } from "@/store/ui";
import { AssistantSessionProvider } from "./AssistantSessionProvider";

let session: AssistantSessionContextValue;
function Probe() {
  session = useAssistantSession();
  return null;
}

function mount() {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <AssistantSessionProvider screenContext={{ route: "/overview", capabilities: ["core"] }}>
        <Probe />
      </AssistantSessionProvider>
    </QueryClientProvider>,
  );
}

interface StreamCall {
  request_id: string;
  args: Record<string, unknown>;
}

function streamCall(index = -1): StreamCall {
  return transport.stream.mock.calls.at(index)![0] as StreamCall;
}

/** Turns end at once, stored as session `s-1`. */
function answerImmediately() {
  transport.stream.mockResolvedValue({
    kind: "ai.chat",
    schema_version: 1,
    data: { finish_reason: "stop", session_id: "s-1" },
  });
}

async function ask(prompt = "What changed?") {
  await act(async () => {
    session.sendPrompt(prompt);
    await Promise.resolve();
  });
}

describe("AssistantSessionProvider permission mode and tool set", () => {
  beforeEach(() => {
    transport.stream.mockReset();
    transport.invoke.mockReset();
    useUiStore.setState({
      assistantModelSelection: { provider: "ollama", model: "qwen3" },
      assistantDefaultPermissionMode: "auto",
      assistantChatPermissionModes: {},
      assistantProviderToolProfiles: {},
    });
  });
  afterEach(cleanup);

  it("sends all tools by default and the tool set the chat picks", async () => {
    answerImmediately();
    mount();
    expect(session.toolProfile).toBe("full");
    await ask();
    expect(streamCall().args).toMatchObject({ tool_profile: "full", permission_mode: "auto" });

    act(() => session.setToolProfile?.("core"));
    expect(session.toolProfile).toBe("core");
    await ask();
    expect(streamCall().args.tool_profile).toBe("core");
    // The chat's pick leaves the provider's own setting alone.
    expect(useUiStore.getState().assistantProviderToolProfiles).toEqual({});
  });

  it("starts each chat in its provider's tool set", async () => {
    answerImmediately();
    useUiStore.setState({ assistantProviderToolProfiles: { ollama: "scoped" } });
    mount();
    await ask();
    expect(streamCall().args.tool_profile).toBe("scoped");

    act(() => session.setToolProfile?.("full"));
    await ask();
    expect(streamCall().args.tool_profile).toBe("full");

    // A new chat drops the chat's pick.
    act(() => session.reset());
    await ask();
    expect(streamCall().args.tool_profile).toBe("scoped");
  });

  it("starts new chats in the Settings default and keeps a reopened chat's mode", async () => {
    answerImmediately();
    useUiStore.setState({ assistantDefaultPermissionMode: "approval" });
    mount();
    expect(session.permissionMode).toBe("approval");
    await ask();
    expect(streamCall().args.permission_mode).toBe("approval");
    expect(session.sessionId).toBe("s-1");

    act(() => session.setPermissionMode?.("full_access"));
    await ask();
    expect(streamCall().args.permission_mode).toBe("full_access");
    expect(useUiStore.getState().assistantChatPermissionModes).toEqual({ "s-1": "full_access" });

    // The default changed in Settings: the next new chat starts there.
    useUiStore.setState({ assistantDefaultPermissionMode: "auto" });
    act(() => session.reset());
    expect(session.permissionMode).toBe("auto");

    // Reopening the saved chat brings its own mode back.
    transport.invoke.mockResolvedValue({
      kind: "ui.chat.sessions.get",
      schema_version: 1,
      data: { id: "s-1", messages: [{ role: "user", content: "What changed?" }] },
    });
    await act(async () => {
      await session.resumeSession("s-1");
    });
    expect(session.permissionMode).toBe("full_access");
  });

  it("switches a streaming answer's mode through ai.chat.permission_mode", async () => {
    let finish!: () => void;
    transport.stream.mockImplementation(
      () =>
        new Promise((resolve) => {
          finish = () =>
            resolve({ kind: "ai.chat", schema_version: 1, data: { finish_reason: "stop" } });
        }),
    );
    transport.invoke.mockResolvedValue({
      kind: "ai.chat.permission_mode",
      schema_version: 1,
      data: { permission_mode: "full_access", released_call_ids: [] },
    });
    mount();

    // Not streaming: nothing to switch, the next turn names the mode.
    act(() => session.setPermissionMode?.("approval"));
    expect(transport.invoke).not.toHaveBeenCalled();

    await ask();
    expect(session.isStreaming).toBe(true);
    const turn = streamCall();
    expect(turn.args.permission_mode).toBe("approval");

    await act(async () => {
      session.setPermissionMode?.("full_access");
      await Promise.resolve();
    });
    expect(transport.invoke).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "ai.chat.permission_mode",
        args: { target_request_id: turn.request_id, permission_mode: "full_access" },
      }),
    );
    expect(session.permissionMode).toBe("full_access");
    await act(async () => {
      finish();
      await Promise.resolve();
    });
  });
});
