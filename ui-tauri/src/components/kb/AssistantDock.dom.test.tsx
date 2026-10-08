// @vitest-environment happy-dom
//
// Mounted: the minimized-while-streaming dock shows a slim follow-up composer
// that queues prompts. A staged attachment travels with that queued prompt, so
// the slim composer must show it and must not ArrowUp-recall an old prompt
// into a composer that already carries a file.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import type * as React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const session = vi.hoisted(() => ({
  attachment: null as { token: string; filename: string } | null,
  clearAttachment: vi.fn(),
  sendPrompt: vi.fn(),
}));

vi.hoisted(() => {
  // Both UI stores persist through web storage, which this DOM does not wire
  // up as globals; zustand resolves them at module evaluation.
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
  useAssistantSession: () => ({
    messages: [
      { id: "u1", role: "user", content: "an earlier question", status: "done" },
      { id: "a1", role: "assistant", content: "", status: "streaming" },
    ],
    isStreaming: true,
    abort: vi.fn(),
    error: null,
    pendingConsent: null,
    queuedPrompts: [],
    sendConsent: vi.fn(),
    selection: { provider: "ollama", model: "qwen3:8b" },
    setSelection: vi.fn(),
    thinkingEffort: "auto",
    setThinkingEffort: vi.fn(),
    sendPrompt: session.sendPrompt,
    reset: vi.fn(),
    branchFromMessage: vi.fn(),
    attachment: session.attachment,
    attachFile: vi.fn(),
    clearAttachment: session.clearAttachment,
  }),
}));

vi.mock("@/components/ai/useReasoningEffortSupport", () => ({
  useSupportedReasoningEffort: () => false,
}));

vi.mock("@/components/ai/ProviderModelPicker", () => ({
  ProviderModelPicker: () => null,
}));

vi.mock("@/components/ai/ChatThread", () => ({ ChatThread: () => null }));
vi.mock("@/components/ai/ToolConsentDialog", () => ({
  ToolConsentDialog: () => null,
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...props }: React.ComponentProps<"a">) => (
    <a {...props}>{children}</a>
  ),
}));

import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useUiStore } from "@/store/ui";
import { AssistantDock } from "./AssistantDock";

function field(): HTMLTextAreaElement {
  return screen.getByRole("textbox") as HTMLTextAreaElement;
}

describe("AssistantDock minimized while streaming", () => {
  beforeEach(() => {
    session.attachment = null;
    session.clearAttachment.mockReset();
    useUiStore.setState({
      assistantDockMinimized: true,
      assistantDockDiscovered: true,
    });
    useAssistantDraftStore.setState({ draft: "" });
  });
  afterEach(cleanup);

  it("recalls earlier prompts into the slim follow-up composer", () => {
    render(<AssistantDock />);
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("an earlier question");
  });

  it("shows the staged attachment and does not recall over it", () => {
    session.attachment = { token: "stage-1", filename: "export.csv" };
    render(<AssistantDock />);
    expect(screen.getByText("export.csv")).toBeTruthy();
    fireEvent.keyDown(field(), { key: "ArrowUp" });
    expect(field().value).toBe("");
    fireEvent.click(screen.getByRole("button", { name: "Remove attachment" }));
    expect(session.clearAttachment).toHaveBeenCalledOnce();
  });
});
