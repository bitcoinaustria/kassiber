// @vitest-environment happy-dom
//
// Mounted: the assistant takes its own layout space. Closed, a bar sits under
// the page in the same column; open, a panel sits beside the page column (no
// overlay) and remembers its width; narrow windows get an overlay sheet.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type * as React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const session = vi.hoisted(() => ({
  messages: [] as Array<{ id: string; role: string; content: string; status: string }>,
  pendingConsent: null as unknown,
  sendPrompt: vi.fn(),
  sendBlockedReason: null as string | null,
}));
const media = vi.hoisted(() => ({ narrow: false }));

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
  useAssistantSession: () => ({
    messages: session.messages,
    isStreaming: false,
    abort: vi.fn(),
    error: null,
    pendingConsent: session.pendingConsent,
    queuedPrompts: [],
    sendConsent: vi.fn(),
    selection: { provider: "copilot", model: "default" },
    setSelection: vi.fn(),
    thinkingEffort: "auto",
    setThinkingEffort: vi.fn(),
    fastMode: false,
    setFastMode: vi.fn(),
    sendPrompt: session.sendPrompt,
    sendBlockedReason: session.sendBlockedReason,
    reset: vi.fn(),
    branchFromMessage: vi.fn(),
    attachment: null,
    attachFile: vi.fn(),
    clearAttachment: vi.fn(),
    focus: null,
    clearFocus: vi.fn(),
  }),
}));
vi.mock("@/components/ai/useReasoningEffortSupport", () => ({
  useSupportedReasoningEffort: () => false,
}));
vi.mock("@/components/ai/ProviderModelPicker", () => ({
  ProviderModelPicker: () => null,
}));
vi.mock("@/components/ai/ChatThread", () => ({
  ChatThread: () => <div data-testid="thread" />,
}));
vi.mock("@/components/ai/ToolConsentDialog", () => ({
  ToolConsentDialog: ({ request }: { request: unknown }) =>
    request ? <div role="alertdialog">consent</div> : null,
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...props }: React.ComponentProps<"a">) => (
    <a {...props}>{children}</a>
  ),
}));

import { useAssistantDraftStore } from "@/store/assistantDraft";
import {
  ASSISTANT_PANEL_DEFAULT_WIDTH,
  ASSISTANT_PANEL_MAX_WIDTH,
  useUiStore,
} from "@/store/ui";
import { AssistantLayout } from "./AssistantLayout";

function mount(enabled = true) {
  return render(
    <AssistantLayout enabled={enabled}>
      <main id="app-main" className="min-h-0 flex-1 overflow-auto">
        page
      </main>
    </AssistantLayout>,
  );
}

function storedUi(): Record<string, unknown> {
  return JSON.parse(localStorage.getItem("kb.ui") ?? "{}").state ?? {};
}

function panel(): HTMLElement | null {
  return document.querySelector("aside[data-assistant-panel]");
}

function pressShortcut() {
  fireEvent.keyDown(window, { key: "i", ctrlKey: true });
}

describe("AssistantLayout", () => {
  beforeEach(() => {
    session.messages = [];
    session.pendingConsent = null;
    session.sendBlockedReason = null;
    useAssistantDraftStore.setState({ draft: "" });
    media.narrow = false;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: (query: string) => ({
        matches: media.narrow,
        media: query,
        addEventListener: () => {},
        removeEventListener: () => {},
      }),
    });
    useUiStore.setState({
      assistantPanelOpen: false,
      assistantPanelWidth: ASSISTANT_PANEL_DEFAULT_WIDTH,
      assistantClosedBar: "composer",
      assistantDockPosition: "right",
    });
  });
  afterEach(cleanup);

  it("keeps the closed bar in the page column, below the page", () => {
    mount();
    const main = screen.getByRole("main");
    const bar = document.querySelector<HTMLElement>("[data-assistant-closed-bar]");
    expect(bar).not.toBeNull();
    // Same flex column, after the page: it takes height instead of floating.
    expect(bar!.parentElement).toBe(main.parentElement);
    expect(main.nextElementSibling).toBe(bar);
    expect(main.parentElement!.className).toContain("flex-col");
    expect(bar!.className).not.toMatch(/\b(absolute|fixed)\b/);
    expect(panel()).toBeNull();
  });

  it("opens beside the page with the shortcut and closes again", () => {
    mount();
    act(pressShortcut);
    const aside = panel();
    expect(aside?.dataset.assistantPanel).toBe("docked");
    expect(aside!.className).not.toMatch(/\b(absolute|fixed)\b/);
    // The page column and the panel are siblings in one row: the page shrinks.
    const column = screen.getByRole("main").parentElement!;
    expect(aside!.parentElement).toBe(column.parentElement);
    expect(column.nextElementSibling).toBe(aside);
    expect(aside!.style.width).toBe(`${ASSISTANT_PANEL_DEFAULT_WIDTH}px`);
    // The composer moved into the panel; the closed bar is gone.
    expect(document.querySelector("[data-assistant-closed-bar]")).toBeNull();
    expect(storedUi().assistantPanelOpen).toBe(true);

    act(pressShortcut);
    expect(panel()).toBeNull();
    expect(storedUi().assistantPanelOpen).toBe(false);
  });

  it("closes from its header button", () => {
    useUiStore.setState({ assistantPanelOpen: true });
    mount();
    fireEvent.click(screen.getByRole("button", { name: /Close assistant panel/ }));
    expect(panel()).toBeNull();
  });

  it("resizes from its left edge and remembers the width", () => {
    useUiStore.setState({ assistantPanelOpen: true });
    const view = mount();
    const handle = screen.getByRole("separator", { name: "Resize assistant panel" });

    fireEvent.pointerDown(handle, { button: 0, clientX: 900, pointerId: 1 });
    fireEvent.pointerMove(handle, { clientX: 800, pointerId: 1 });
    const dragged = ASSISTANT_PANEL_DEFAULT_WIDTH + 100;
    expect(panel()!.style.width).toBe(`${dragged}px`);
    // The store only commits once the drag ends.
    expect(useUiStore.getState().assistantPanelWidth).toBe(ASSISTANT_PANEL_DEFAULT_WIDTH);
    fireEvent.pointerUp(handle, { clientX: 800, pointerId: 1 });
    expect(useUiStore.getState().assistantPanelWidth).toBe(dragged);
    expect(storedUi().assistantPanelWidth).toBe(dragged);

    fireEvent.pointerDown(handle, { button: 0, clientX: 800, pointerId: 2 });
    fireEvent.pointerMove(handle, { clientX: -2000, pointerId: 2 });
    fireEvent.pointerUp(handle, { clientX: -2000, pointerId: 2 });
    expect(useUiStore.getState().assistantPanelWidth).toBe(ASSISTANT_PANEL_MAX_WIDTH);

    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(useUiStore.getState().assistantPanelWidth).toBe(ASSISTANT_PANEL_MAX_WIDTH - 16);
    expect(handle.getAttribute("aria-valuenow")).toBe(String(ASSISTANT_PANEL_MAX_WIDTH - 16));

    // A remount (or relaunch) restores the remembered width.
    view.unmount();
    mount();
    expect(panel()!.style.width).toBe(`${ASSISTANT_PANEL_MAX_WIDTH - 16}px`);
  });

  it("overlays as a sheet on narrow windows", () => {
    media.narrow = true;
    useUiStore.setState({ assistantPanelOpen: true });
    mount();
    const aside = panel();
    expect(aside?.dataset.assistantPanel).toBe("overlay");
    expect(aside!.className).toContain("absolute");
    expect(screen.queryByRole("separator")).toBeNull();
    fireEvent.click(screen.getAllByRole("button", { name: "Close assistant panel" })[0]);
    expect(panel()).toBeNull();
  });

  it("opens from the launcher and focuses the panel composer", () => {
    useUiStore.setState({ assistantClosedBar: "launcher" });
    mount();
    expect(screen.queryByRole("textbox")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open assistant panel" }));
    expect(panel()).not.toBeNull();
    expect(document.activeElement).toBe(screen.getByRole("textbox"));
  });

  it("holds sending in both composers while the book's provider settings load", () => {
    const reason = "Loading this book's AI provider settings…";
    session.sendBlockedReason = reason;
    useAssistantDraftStore.setState({ draft: "Why 11 pending?" });
    const view = mount();
    // The compact composer keeps a send button inline and one in its
    // focused toolbar; both say why sending waits.
    const closedSend = screen.getAllByRole("button", { name: reason });
    expect(closedSend.length).toBeGreaterThan(0);
    for (const button of closedSend) {
      expect((button as HTMLButtonElement).disabled).toBe(true);
    }

    act(() => useUiStore.setState({ assistantPanelOpen: true }));
    view.rerender(
      <AssistantLayout enabled>
        <main id="app-main" className="min-h-0 flex-1 overflow-auto">
          page
        </main>
      </AssistantLayout>,
    );
    const panelSend = screen.getByRole("button", { name: reason });
    expect(panel()!.contains(panelSend)).toBe(true);
    expect((panelSend as HTMLButtonElement).disabled).toBe(true);
    expect(session.sendPrompt).not.toHaveBeenCalled();
  });

  it("shows nothing below the page when the bar is hidden", () => {
    useUiStore.setState({ assistantClosedBar: "hidden" });
    mount();
    expect(document.querySelector("[data-assistant-closed-bar]")).toBeNull();
  });

  it("opens itself for a consent prompt", () => {
    session.pendingConsent = { callId: "c1", name: "ui.journals.process" };
    mount();
    expect(panel()).not.toBeNull();
    expect(screen.getByRole("alertdialog")).toBeTruthy();
  });

  it("stays out of the full assistant page", () => {
    useUiStore.setState({ assistantPanelOpen: true });
    mount(false);
    expect(panel()).toBeNull();
    expect(document.querySelector("[data-assistant-closed-bar]")).toBeNull();
    act(pressShortcut);
    expect(panel()).toBeNull();
  });
});
