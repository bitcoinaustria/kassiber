// @vitest-environment happy-dom
//
// A handoff parked in the composer (made before a model was chosen) keeps its
// page's context only while the draft is still the user's edit of its text.
// A starter question the composer puts in the draft's place is an unrelated
// question: the parked context goes, so it is not sent with the other page's
// tool pack.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AssistantSessionContextValue } from "@/components/ai/assistantSession";

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

vi.mock("@/components/ai/assistantSession", () => ({
  useAssistantSession: (): Partial<AssistantSessionContextValue> => ({
    messages: [],
    isStreaming: false,
    error: null,
    pendingConsents: [],
    queuedPrompts: [],
    selection: null,
    thinkingEffort: "auto",
    fastMode: false,
    returnPath: "/overview",
    sessionId: null,
    incognito: false,
    attachments: [],
    setSelection: vi.fn(),
    setThinkingEffort: vi.fn(),
    setFastMode: vi.fn(),
    setIncognito: vi.fn(),
    attachFile: vi.fn(),
    removeAttachment: vi.fn(),
    sendPrompt: vi.fn(),
    sendConsent: vi.fn(),
    abort: vi.fn(),
    reset: vi.fn(),
    resumeSession: vi.fn(),
    branchFromMessage: vi.fn(),
    editUserMessage: vi.fn(),
    forgetSession: vi.fn(),
    focus: null,
    clearFocus: vi.fn(),
  }),
}));
vi.mock("@/components/ai/ChatHistoryPanel", () => ({ ChatHistoryPanel: () => null }));
vi.mock("@/components/ai/ChatThread", () => ({ ChatThread: () => null }));
vi.mock("@/components/ai/RecentChats", () => ({ RecentChats: () => null }));
vi.mock("@/components/ai/ProviderModelPicker", () => ({ ProviderModelPicker: () => null }));
vi.mock("@/components/ai/useReasoningEffortSupport", () => ({
  useSupportedReasoningEffort: () => false,
}));
// The Incognito tooltip reads the provider list; not what this covers.
vi.mock("@/components/ai/incognitoScope", () => ({ useIncognitoScope: () => null }));

import { useAssistantDraftStore } from "@/store/assistantDraft";
import { Assistant } from "./Assistant";

const FIX_PROMPT = "Fix the cause that holds this transaction in quarantine.";
const QUARANTINE = { route: "/quarantine" as const, capabilities: ["review" as const] };

function parkQuarantineFix() {
  // "Fix with Assistant" with no model chosen yet.
  act(() =>
    useAssistantDraftStore
      .getState()
      .parkHandoff(FIX_PROMPT, { context: QUARANTINE, book: { daemonSession: 0, scope: { workspace_id: "ws-1", profile_id: "profile-1" } } }),
  );
}

describe("a parked handoff on the assistant page", () => {
  beforeEach(() => {
    useAssistantDraftStore.setState({ draft: "", handoff: null });
  });
  afterEach(cleanup);

  it("keeps its context through the user's own edits", () => {
    parkQuarantineFix();
    render(<Assistant />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: `${FIX_PROMPT} Please explain each step.` },
    });
    expect(useAssistantDraftStore.getState().handoff?.context).toEqual(QUARANTINE);
  });

  it("drops it when a starter question replaces the text", () => {
    parkQuarantineFix();
    render(<Assistant />);
    fireEvent.click(screen.getByRole("button", { name: "Prepare tax export" }));

    const { draft, handoff } = useAssistantDraftStore.getState();
    expect(draft).toBe(
      "Prepare the Austrian tax export checklist and call out missing journal, pricing, or report prerequisites.",
    );
    // Sent later, it is an ordinary question with the report tools, not a
    // quarantine fix limited to review tools.
    expect(handoff).toBeNull();
  });
});
