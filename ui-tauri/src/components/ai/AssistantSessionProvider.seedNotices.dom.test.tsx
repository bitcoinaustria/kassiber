// @vitest-environment happy-dom
//
// A branched or edited chat stores the notices of the answers it copies: they
// travel beside the messages as `seedNotices`, never inside the model prompt.
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AiChatMessage, AiChatRequest } from "@/daemon/stream";

const conversation: AiChatMessage[] = [
  { id: "u1", role: "user", content: "First question", status: "done" },
  {
    id: "a1",
    role: "assistant",
    content: "First answer",
    status: "done",
    notices: ["fast_mode_unavailable"],
  },
  { id: "u2", role: "user", content: "Second question", status: "done" },
  { id: "a2", role: "assistant", content: "Second answer", status: "done" },
];

const mocks = vi.hoisted(() => ({ send: vi.fn(), messages: [] as unknown[] }));

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

vi.mock("@/daemon/stream", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/stream")>();
  return {
    ...actual,
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
  };
});

vi.mock("@/components/ai/useFastModeSupport", () => ({
  useFastModeSupport: () => ({ supported: false, resolved: true, runtime: "claude" }),
}));

import {
  useAssistantSession,
  type AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import { buildAiChatStreamArgs } from "@/daemon/stream";
import { useUiStore } from "@/store/ui";
import { AssistantSessionProvider } from "./AssistantSessionProvider";

let session: AssistantSessionContextValue;
function Probe() {
  session = useAssistantSession();
  return null;
}

function lastRequest(): AiChatRequest {
  return mocks.send.mock.calls.at(-1)?.[0] as AiChatRequest;
}

describe("seeded answer notices", () => {
  beforeEach(() => {
    mocks.send.mockReset();
    mocks.messages = conversation;
    useUiStore.setState({
      assistantModelSelection: { provider: "claude", model: "opus" },
    });
    render(
      <AssistantSessionProvider screenContext={{ route: "/overview", capabilities: ["core"] }}>
        <Probe />
      </AssistantSessionProvider>,
    );
  });
  afterEach(cleanup);

  it("sends a branch's copied notices beside the messages, not in them", () => {
    act(() => session.branchFromMessage("a2"));
    act(() => session.sendPrompt("Branched prompt"));
    const request = lastRequest();
    expect(request.seedHistory).toBe(true);
    expect(request.seedNotices).toEqual([{ index: 1, notices: ["fast_mode_unavailable"] }]);
    // The model's messages are role and content only.
    for (const message of request.messages) {
      expect(Object.keys(message).sort()).toEqual(["content", "role"]);
    }
    expect(buildAiChatStreamArgs(request).seed_notices).toEqual([
      { index: 1, notices: ["fast_mode_unavailable"] },
    ]);
  });

  it("carries them through an edited prompt too", () => {
    act(() => session.editUserMessage("u2", "Edited question"));
    const request = lastRequest();
    expect(request.seedHistory).toBe(true);
    expect(request.seedNotices).toEqual([{ index: 1, notices: ["fast_mode_unavailable"] }]);
  });

  it("sends none for a turn that does not seed a new session", () => {
    act(() => session.sendPrompt("Ordinary follow-up"));
    const request = lastRequest();
    expect(request.seedHistory).toBe(false);
    expect(request.seedNotices).toBeUndefined();
    expect(
      buildAiChatStreamArgs({ ...request, seedNotices: [{ index: 1, notices: ["fast_mode_unavailable"] }] })
        .seed_notices,
    ).toBeUndefined();
  });
});
