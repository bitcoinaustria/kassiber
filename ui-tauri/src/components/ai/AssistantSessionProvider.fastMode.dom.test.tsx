// @vitest-environment happy-dom
//
// Fast mode reaches the chat request only while the selected model offers it,
// and a switch to a model without it drops the request.
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  send: vi.fn(),
  fastSupported: true,
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

vi.mock("@/components/ai/useFastModeSupport", () => ({
  useFastModeSupport: () => ({
    supported: mocks.fastSupported,
    resolved: true,
    runtime: "codex",
  }),
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
    <AssistantSessionProvider screenContext={{ route: "/overview", capabilities: ["core"] }}>
      <Probe />
    </AssistantSessionProvider>,
  );
}

function lastOptions(): unknown {
  return mocks.send.mock.calls.at(-1)?.[0]?.options;
}

describe("AssistantSessionProvider fast mode", () => {
  beforeEach(() => {
    mocks.send.mockReset();
    mocks.fastSupported = true;
    useUiStore.setState({
      assistantModelSelection: { provider: "codex", model: "gpt-5.4" },
    });
  });
  afterEach(cleanup);

  it("sends fast_mode only once it is switched on", () => {
    mount();
    act(() => session.sendPrompt("Reply with just: ok"));
    expect(lastOptions()).toBeUndefined();

    act(() => session.setFastMode(true));
    expect(session.fastMode).toBe(true);
    act(() => session.sendPrompt("Reply with just: ok"));
    expect(lastOptions()).toEqual({ fast_mode: true });

    act(() => session.setThinkingEffort("high"));
    act(() => session.sendPrompt("Reply with just: ok"));
    expect(lastOptions()).toEqual({ reasoning_effort: "high", fast_mode: true });
  });

  it("drops fast mode when the selected model does not offer it", () => {
    const view = mount();
    act(() => session.setFastMode(true));
    expect(session.fastMode).toBe(true);

    mocks.fastSupported = false;
    act(() => {
      useUiStore.setState({
        assistantModelSelection: { provider: "copilot", model: "default" },
      });
    });
    view.rerender(
      <AssistantSessionProvider screenContext={{ route: "/overview", capabilities: ["core"] }}>
        <Probe />
      </AssistantSessionProvider>,
    );
    expect(session.fastMode).toBe(false);
    act(() => session.sendPrompt("Reply with just: ok"));
    expect(lastOptions()).toBeUndefined();

    // Back on a fast-capable model, the request does not silently return.
    mocks.fastSupported = true;
    act(() => {
      useUiStore.setState({
        assistantModelSelection: { provider: "codex", model: "gpt-5.4" },
      });
    });
    expect(session.fastMode).toBe(false);
  });
});
