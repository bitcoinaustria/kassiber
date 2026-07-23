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

vi.mock("@/components/ai/RecentChats", () => ({
  RecentChats: () => null,
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
    branchFromMessage: vi.fn(),
    editUserMessage: vi.fn(),
    forgetSession: vi.fn(),
    ...overrides,
  };
}

describe("Assistant toolbar", () => {
  it("always exposes the more-actions menu", () => {
    assistantSession = makeAssistantSession();

    const html = renderToStaticMarkup(<Assistant />);

    // Incognito/History/Clear now live behind the collapsed "…" menu, so the
    // trigger is present even though its portaled items are not in the DOM
    // until it opens.
    expect(html).toContain("More actions");
  });

  it("shows Export only once a conversation exists", () => {
    assistantSession = makeAssistantSession();

    expect(renderToStaticMarkup(<Assistant />)).not.toContain("Export");

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

    expect(renderToStaticMarkup(<Assistant />)).toContain("Export");
  });

  it("keeps the toolbar full-width while the thread column stays capped", () => {
    assistantSession = makeAssistantSession({
      messages: [
        {
          id: "message-1",
          role: "user",
          content: "Hello",
          status: "done",
        },
      ],
    });

    const html = renderToStaticMarkup(<Assistant />);

    // The conversation column + docked composer are capped (ported to the
    // T3Code max-w-3xl reading column) while the toolbar row spans full width.
    expect(html).not.toContain("max-w-6xl");
    expect(html).not.toContain("max-w-4xl");
    expect(html).toContain("max-w-3xl");
  });
});
