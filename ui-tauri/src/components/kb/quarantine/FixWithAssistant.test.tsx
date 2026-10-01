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

function renderActions(session: Partial<AssistantSessionContextValue> | null, count = 5) {
  return renderToStaticMarkup(<AssistantSessionContext.Provider value={session as AssistantSessionContextValue | null}>
    <FixWithAssistant attentionCount={count} />
  </AssistantSessionContext.Provider>);
}

describe("fixing quarantine with the assistant", () => {
  beforeEach(() => { buttons.length = 0; useAssistantDraftStore.getState().setDraft(""); });

  it("starts one scoped fix in chat and expands the existing dock", () => {
    const sendPrompt = vi.fn();
    renderActions({ sendPrompt, selection: { provider: "local", model: "model" }, isStreaming: false });
    buttons[0].onClick?.();
    expect(sendPrompt).toHaveBeenCalledOnce();
    expect(sendPrompt.mock.calls[0][0]).toContain("5 quarantine causes");
    expect(sendPrompt.mock.calls[0][0]).toContain("confirm once");
    expect(useUiStore.getState().assistantDockMinimized).toBe(false);
    expect(useUiStore.getState().assistantDockExpanded).toBe(true);
  });

  it("preserves the prompt in the composer when a model must first be selected", () => {
    const sendPrompt = vi.fn();
    renderActions({ sendPrompt, selection: null, isStreaming: false });
    buttons[0].onClick?.();
    expect(sendPrompt).not.toHaveBeenCalled();
    expect(useAssistantDraftStore.getState().draft).toContain("5 quarantine causes");
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
