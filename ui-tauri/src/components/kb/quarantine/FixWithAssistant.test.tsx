import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import "@/i18n";
import { AssistantSessionContext, type AssistantSessionContextValue } from "@/components/ai/assistantSession";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useUiStore } from "@/store/ui";

const buttons = vi.hoisted(() => [] as Array<{ onClick?: () => void; disabled?: boolean }>);
vi.mock("@/components/ui/button", () => ({ Button: (props: ComponentProps<"button"> & { variant?: string }) => {
  buttons.push(props as { onClick?: () => void; disabled?: boolean });
  return <button disabled={props.disabled}>{props.children}</button>;
} }));
import { FixWithAssistant } from "./FixWithAssistant";

// The book open when the user clicks, which the handoff carries.
const BOOK = { daemonSession: 0, scope: { workspace_id: "ws-1", profile_id: "profile-1" } };

function renderActions(session: Partial<AssistantSessionContextValue> | null, count = 5) {
  const value = session && { captureBook: () => BOOK, ...session };
  return renderToStaticMarkup(<AssistantSessionContext.Provider value={value as AssistantSessionContextValue | null}>
    <FixWithAssistant attentionCount={count} />
  </AssistantSessionContext.Provider>);
}

describe("fixing quarantine with the assistant", () => {
  beforeEach(() => { buttons.length = 0; useAssistantDraftStore.getState().setDraft(""); });

  it("starts one scoped fix in chat and opens the assistant panel", () => {
    const sendPrompt = vi.fn();
    renderActions({ sendPrompt, selection: { provider: "local", model: "model" }, isStreaming: false });
    buttons[0].onClick?.();
    expect(sendPrompt).toHaveBeenCalledOnce();
    expect(sendPrompt.mock.calls[0][0]).toContain("5 quarantine causes");
    expect(sendPrompt.mock.calls[0][0]).toContain("confirm once");
    // A handoff: a pinned cause cannot narrow the fix to one transaction.
    expect(sendPrompt.mock.calls[0][1]).toMatchObject({ book: BOOK });
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
  });

  it("hands the fix over as a handoff when a model must first be selected", () => {
    const sendPrompt = vi.fn();
    renderActions({ sendPrompt, selection: null, isStreaming: false });
    buttons[0].onClick?.();
    // The session parks it in the composer, with this page and book, until a
    // model is chosen (covered with the session provider).
    expect(sendPrompt).toHaveBeenCalledOnce();
    expect(sendPrompt.mock.calls[0][0]).toContain("5 quarantine causes");
    expect(sendPrompt.mock.calls[0][1]).toEqual(expect.any(Object));
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
  });

  it("does not start another run while one is streaming, and hides the action when AI is disabled", () => {
    const sendPrompt = vi.fn();
    renderActions({ sendPrompt, selection: { provider: "local", model: "model" }, isStreaming: true });
    expect(buttons[0].disabled).toBe(true);
    buttons[0].onClick?.();
    expect(sendPrompt).not.toHaveBeenCalled();
    expect(renderActions(null)).toBe("");
  });

  it("offers nothing when nothing needs the user", () => {
    expect(renderActions({ sendPrompt: vi.fn(), selection: { provider: "local", model: "model" }, isStreaming: false }, 0)).toBe("");
  });
});
