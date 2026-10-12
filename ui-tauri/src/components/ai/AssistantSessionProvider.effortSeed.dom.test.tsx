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
    messages: [],
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
});
