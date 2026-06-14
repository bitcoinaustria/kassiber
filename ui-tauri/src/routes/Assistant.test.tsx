import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { AssistantSessionContextValue } from "@/components/ai/assistantSession";

let assistantSession: AssistantSessionContextValue;

vi.mock("@/components/ai/assistantSession", () => ({
  useAssistantSession: () => assistantSession,
}));

vi.mock("@/components/ai/ChatHistoryPanel", () => ({
  ChatHistoryPanel: () => <button type="button">History</button>,
}));

vi.mock("@/components/ai/ChatThread", () => ({
  ChatThread: () => <div>Conversation</div>,
}));

vi.mock("@/components/ai/ToolConsentDialog", () => ({
  ToolConsentDialog: () => null,
}));

vi.mock("@/components/ai/useReasoningEffortSupport", () => ({
  useSupportedReasoningEffort: () => false,
}));

vi.mock("@/components/ai-02", () => ({
  default: () => <form>Composer</form>,
}));

import { Assistant } from "./Assistant";

function makeAssistantSession(
  overrides: Partial<AssistantSessionContextValue> = {},
): AssistantSessionContextValue {
  return {
    messages: [],
    isStreaming: false,
    error: null,
    pendingConsent: null,
    queuedPrompts: [],
    selection: { provider: "ollama", model: "gemma" },
    thinkingEffort: "auto",
    returnPath: "/overview",
    sessionId: null,
    incognito: false,
    setSelection: vi.fn(),
    setThinkingEffort: vi.fn(),
    setIncognito: vi.fn(),
    sendPrompt: vi.fn(),
    sendConsent: vi.fn(),
    abort: vi.fn(),
    reset: vi.fn(),
    resumeSession: vi.fn(),
    forgetSession: vi.fn(),
    ...overrides,
  };
}

describe("Assistant toolbar", () => {
  it("shows the incognito toggle for a fresh chat", () => {
    assistantSession = makeAssistantSession();

    const html = renderToStaticMarkup(<Assistant />);

    expect(html).toContain("Incognito");
  });

  it("hides the incognito toggle for an existing saved chat", () => {
    assistantSession = makeAssistantSession({
      sessionId: "chat-session-1",
      messages: [
        {
          id: "message-1",
          role: "user",
          content: "Resume this chat",
          status: "done",
        },
      ],
    });

    const html = renderToStaticMarkup(<Assistant />);

    expect(html).not.toContain("Incognito");
  });
});
