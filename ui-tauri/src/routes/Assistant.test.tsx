import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import type { AssistantSessionContextValue } from "@/components/ai/assistantSession";
import { incognitoScope, type IncognitoScope } from "@/components/ai/incognitoScope";
import i18n from "@/i18n";

let assistantSession: AssistantSessionContextValue;
let selectedIncognitoScope: IncognitoScope | null = null;

vi.mock("@/components/ai/assistantSession", () => ({
  useAssistantSession: () => assistantSession,
}));

vi.mock("@/components/ai/incognitoScope", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/ai/incognitoScope")>()),
  useIncognitoScope: () => selectedIncognitoScope,
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
    fastMode: false,
    returnPath: "/overview",
    sessionId: null,
    incognito: false,
    attachment: null,
    setSelection: vi.fn(),
    setThinkingEffort: vi.fn(),
    setFastMode: vi.fn(),
    setIncognito: vi.fn(),
    attachFile: vi.fn(),
    clearAttachment: vi.fn(),
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

describe("Incognito scope", () => {
  it("classifies what each kind of provider keeps", () => {
    expect(incognitoScope({ base_url: "http://localhost:11434/v1", kind: "local" })).toBe("local");
    expect(incognitoScope({ base_url: "https://api.example/v1", kind: "remote" })).toBe("remote");
    expect(incognitoScope({ base_url: "https://tee.example/v1", kind: "tee" })).toBe("remote");
    for (const locator of ["codex-cli://default", "claude-cli://default", "opencode-cli://default"]) {
      expect(incognitoScope({ base_url: locator, kind: "remote" })).toBe("cli");
    }
    expect(incognitoScope({ base_url: "copilot-cli://default", kind: "remote" })).toBe("copilot");
    expect(incognitoScope(undefined)).toBeNull();
  });

  it.each([
    ["en", "copilot", "Incognito is on - this conversation is not stored. GitHub Copilot keeps it only for the current turn, in a temporary folder. Click to turn off."],
    ["en", "local", "Incognito is on - this conversation is not stored. Nothing is stored. Click to turn off."],
    ["de", "copilot", "Incognito ist aktiv - diese Unterhaltung wird nicht gespeichert. GitHub Copilot behält sie nur für den aktuellen Durchgang, in einem temporären Ordner. Klicken zum Ausschalten."],
  ] as const)("states the %s residual for a %s provider on the active marker", async (language, scope, title) => {
    await i18n.changeLanguage(language);
    try {
      selectedIncognitoScope = scope;
      assistantSession = makeAssistantSession({ incognito: true });
      const html = renderToStaticMarkup(<Assistant />);
      expect(html).toContain(`title="${title}"`);
    } finally {
      selectedIncognitoScope = null;
      await i18n.changeLanguage("en");
    }
  });

  it("keeps the plain wording until the provider list is known", () => {
    selectedIncognitoScope = null;
    assistantSession = makeAssistantSession({ incognito: true });
    expect(renderToStaticMarkup(<Assistant />)).toContain(
      'title="Incognito is on - this conversation is not stored. Click to turn off."',
    );
  });
});
