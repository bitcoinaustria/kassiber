// @vitest-environment happy-dom
//
// Until the open book's provider settings have seeded the reasoning effort
// (the provider list is still loading, e.g. right after a book switch), a
// prompt is held instead of being sent with the previous book's effort or
// auto, and the composer says why.
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  send: vi.fn(),
  seeded: false,
  setEffort: null as ((effort: string) => void) | null,
  messages: [] as Array<{ id: string; role: string; content: string }>,
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

vi.mock("@/daemon/stream", () => ({
  useAiChatStream: () => ({
    messages: mocks.messages,
    isStreaming: false,
    error: null,
    pendingConsent: null,
    sessionId: null,
    send: mocks.send,
    sendConsent: vi.fn(),
    abort: vi.fn(),
    reset: vi.fn(),
    loadConversation: vi.fn(),
    forgetSession: vi.fn(),
  }),
}));

// Stands in for the seeding hook: the test decides when seeding is done.
// The open book's ids, which every turn carries for the daemon's check.
vi.mock("@/components/ai/useActiveBookScope", () => ({
  useActiveBookScope: () => ({ workspace_id: "ws-1", profile_id: "profile-1" }),
}));

vi.mock("@/components/ai/useProviderDefaultEffort", () => ({
  useProviderDefaultEffort: ({
    setThinkingEffort,
  }: {
    setThinkingEffort: (effort: string) => void;
  }) => {
    mocks.setEffort = setThinkingEffort;
    return { setThinkingEffort, effortSeeded: mocks.seeded };
  },
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

function tree() {
  return (
    <AssistantSessionProvider screenContext={{ route: "/overview", capabilities: ["core"] }}>
      <Probe />
    </AssistantSessionProvider>
  );
}

describe("AssistantSessionProvider before the effort is seeded", () => {
  beforeEach(() => {
    mocks.send.mockReset();
    mocks.seeded = false;
    mocks.messages = [];
    useUiStore.setState({
      assistantModelSelection: { provider: "claude", model: "opus" },
    });
  });
  afterEach(cleanup);

  it("holds a prompt until seeding is done, then sends it with the seeded effort", () => {
    const view = render(tree());
    expect(session.sendBlockedReason).toBeTruthy();

    act(() => session.sendPrompt("Reply with just: ok"));
    expect(mocks.send).not.toHaveBeenCalled();
    expect(session.queuedPrompts).toEqual(["Reply with just: ok"]);

    // The provider list arrives: this book's default seeds the effort.
    mocks.seeded = true;
    act(() => mocks.setEffort?.("low"));
    view.rerender(tree());

    expect(session.sendBlockedReason).toBeNull();
    expect(mocks.send).toHaveBeenCalledOnce();
    expect(mocks.send.mock.calls[0][0].options).toEqual({ reasoning_effort: "low" });
  });

  it("holds an edited prompt's regeneration the same way", () => {
    // A first exchange, asked once the effort was seeded, binds the
    // conversation to this book.
    mocks.seeded = true;
    const view = render(tree());
    act(() => session.sendPrompt("first question"));
    mocks.send.mockReset();
    mocks.messages = [
      { id: "u1", role: "user", content: "first question" },
      { id: "a1", role: "assistant", content: "an answer" },
    ];
    // Then this book's provider settings load again (say after a change).
    mocks.seeded = false;
    view.rerender(tree());

    act(() => session.editUserMessage("u1", "edited question"));
    expect(mocks.send).not.toHaveBeenCalled();
    expect(session.queuedPrompts).toEqual(["edited question"]);

    mocks.seeded = true;
    act(() => mocks.setEffort?.("low"));
    view.rerender(tree());

    expect(mocks.send).toHaveBeenCalledOnce();
    const request = mocks.send.mock.calls[0][0];
    expect(request.messages.at(-1)).toEqual({ role: "user", content: "edited question" });
    expect(request.options).toEqual({ reasoning_effort: "low" });
  });
});
