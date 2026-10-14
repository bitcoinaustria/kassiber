// @vitest-environment happy-dom
//
// Mounted: the assistant takes its own layout space. Closed, a bar sits under
// the page in the same column; open, a panel sits beside the page column (no
// overlay) and remembers its width; narrow windows get an overlay sheet.
import {
  act,
  cleanup,
  fireEvent,
  render,
  renderHook,
  screen,
} from "@testing-library/react";
import type * as React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const session = vi.hoisted(() => ({
  messages: [] as Array<{ id: string; role: string; content: string; status: string }>,
  pendingConsent: null as unknown,
  error: null as { code: string; message: string } | null,
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
    error: session.error,
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
import { shellModalOpen } from "../shellNavigation";
import { AssistantLayout } from "./AssistantLayout";
import { useAssistantColumnClaim } from "./useAssistantColumnClaim";

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
    session.error = null;
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
      assistantColumnClaimed: false,
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

  it("resizes from the width it renders at when the window caps it", () => {
    useUiStore.setState({
      assistantPanelOpen: true,
      assistantPanelWidth: ASSISTANT_PANEL_MAX_WIDTH,
    });
    mount();
    const aside = panel()!;
    // A 1000px row keeps the page its 480px, so CSS renders a remembered
    // 640px panel at 520px.
    Object.defineProperty(aside.parentElement!, "clientWidth", {
      configurable: true,
      value: 1000,
    });
    aside.getBoundingClientRect = () =>
      ({ width: 520, height: 800, top: 0, left: 480, right: 1000, bottom: 800, x: 480, y: 0 }) as DOMRect;
    const handle = screen.getByRole("separator", { name: "Resize assistant panel" });

    // A click on the handle is not a resize: the remembered width stays.
    fireEvent.pointerDown(handle, { button: 0, clientX: 480, pointerId: 1 });
    fireEvent.pointerUp(handle, { clientX: 480, pointerId: 1 });
    expect(useUiStore.getState().assistantPanelWidth).toBe(ASSISTANT_PANEL_MAX_WIDTH);

    // Dragging 20px right narrows the panel by 20px from what is on screen.
    fireEvent.pointerDown(handle, { button: 0, clientX: 480, pointerId: 2 });
    fireEvent.pointerMove(handle, { clientX: 500, pointerId: 2 });
    expect(aside.style.width).toBe("500px");
    fireEvent.pointerUp(handle, { clientX: 500, pointerId: 2 });
    expect(useUiStore.getState().assistantPanelWidth).toBe(500);

    // The keyboard steps from the rendered width too.
    act(() => useUiStore.setState({ assistantPanelWidth: ASSISTANT_PANEL_MAX_WIDTH }));
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(useUiStore.getState().assistantPanelWidth).toBe(520 - 16);
  });

  it("reports the width it renders at when the window narrows", () => {
    const observers: Array<() => void> = [];
    const previousResizeObserver = globalThis.ResizeObserver;
    globalThis.ResizeObserver = class {
        readonly callback: () => void;
        constructor(callback: () => void) {
          this.callback = callback;
        }
        observe() {
          observers.push(this.callback);
        }
        unobserve() {}
        disconnect() {}
      } as unknown as typeof ResizeObserver;
    let rowWidth = 1400;
    const clientWidth = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get(this: HTMLElement) {
        return this.querySelector(":scope > aside[data-assistant-panel]") ? rowWidth : 0;
      },
    });
    try {
      useUiStore.setState({
        assistantPanelOpen: true,
        assistantPanelWidth: ASSISTANT_PANEL_MAX_WIDTH,
      });
      mount();
      act(() => observers.forEach((notify) => notify()));
      const handle = screen.getByRole("separator", { name: "Resize assistant panel" });
      expect(handle.getAttribute("aria-valuenow")).toBe(String(ASSISTANT_PANEL_MAX_WIDTH));

      // Narrowing the window caps the remembered 640px at 520px on screen; the
      // separator follows without waiting for another render.
      rowWidth = 1000;
      act(() => observers.forEach((notify) => notify()));
      expect(handle.getAttribute("aria-valuemax")).toBe("520");
      expect(handle.getAttribute("aria-valuenow")).toBe("520");
    } finally {
      if (clientWidth) Object.defineProperty(HTMLElement.prototype, "clientWidth", clientWidth);
      globalThis.ResizeObserver = previousResizeObserver;
    }
  });

  it("gives the page its cursor and selection back when it closes mid-resize", () => {
    useUiStore.setState({ assistantPanelOpen: true });
    mount();
    const handle = screen.getByRole("separator", { name: "Resize assistant panel" });
    fireEvent.pointerDown(handle, { button: 0, clientX: 900, pointerId: 1 });
    expect(document.body.style.cursor).toBe("col-resize");
    expect(document.body.style.userSelect).toBe("none");

    // Closed with the shortcut before the pointer is released.
    act(pressShortcut);
    expect(panel()).toBeNull();
    expect(document.body.style.cursor).toBe("");
    expect(document.body.style.userSelect).toBe("");
    // The unfinished drag is not remembered as the new width.
    expect(useUiStore.getState().assistantPanelWidth).toBe(ASSISTANT_PANEL_DEFAULT_WIDTH);
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

  it("is a modal sheet on narrow windows and a plain panel otherwise", () => {
    media.narrow = true;
    render(
      <AssistantLayout enabled>
        <main id="app-main" className="min-h-0 flex-1 overflow-auto">
          <button type="button">page action</button>
        </main>
      </AssistantLayout>,
    );
    const pageAction = screen.getByRole("button", { name: "page action" });
    act(() => pageAction.focus());
    act(pressShortcut);

    const sheet = panel()!;
    expect(sheet.getAttribute("role")).toBe("dialog");
    expect(sheet.getAttribute("aria-modal")).toBe("true");
    expect(sheet.getAttribute("aria-label")).toBe("Kassiber assistant");
    // The covered page cannot take focus, and focus moved into the sheet.
    expect(screen.getByRole("main").parentElement!.hasAttribute("inert")).toBe(true);
    expect(sheet.contains(document.activeElement)).toBe(true);

    // Tab and Shift+Tab stay inside the sheet.
    const tabbables = Array.from(
      sheet.querySelectorAll<HTMLElement>("a[href], button:not([disabled]), textarea"),
    );
    const first = tabbables[0];
    const last = tabbables[tabbables.length - 1];
    act(() => last.focus());
    fireEvent.keyDown(last, { key: "Tab" });
    expect(document.activeElement).toBe(first);
    fireEvent.keyDown(first, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(last);

    // Escape closes it wherever focus is, unless another dialog is on top.
    session.pendingConsent = { callId: "c1", name: "ui.journals.process" };
    act(() => useUiStore.setState({ assistantClosedBar: "composer" }));
    act(() => {
      document.body.focus();
      fireEvent.keyDown(document.body, { key: "Escape" });
    });
    expect(panel()).not.toBeNull();
    session.pendingConsent = null;
    act(() => useUiStore.setState({ assistantClosedBar: "launcher" }));
    act(() => {
      fireEvent.keyDown(document.body, { key: "Escape" });
    });
    expect(panel()).toBeNull();
    // The page is usable again, and focus is back where it was.
    expect(screen.getByRole("main").parentElement!.hasAttribute("inert")).toBe(false);
    expect(document.activeElement).toBe(pageAction);
  });

  it("makes the shell around a narrow-window sheet inert too", () => {
    media.narrow = true;
    render(
      <>
        <nav data-shell-chrome>
          <button type="button">sidebar link</button>
        </nav>
        <AssistantLayout enabled>
          <main id="app-main" className="min-h-0 flex-1 overflow-auto">
            page
          </main>
        </AssistantLayout>
      </>,
    );
    const chrome = screen.getByRole("navigation", { hidden: true });
    expect(chrome.hasAttribute("inert")).toBe(false);
    act(pressShortcut);
    // Sidebar and title-bar controls cannot take focus or clicks while the
    // modal sheet is open (the title bar row itself stays a drag region).
    expect(chrome.hasAttribute("inert")).toBe(true);
    act(pressShortcut);
    expect(chrome.hasAttribute("inert")).toBe(false);
  });

  it("keeps the docked panel non-modal", () => {
    useUiStore.setState({ assistantPanelOpen: true });
    mount();
    const docked = panel()!;
    expect(docked.getAttribute("role")).toBeNull();
    expect(docked.getAttribute("aria-modal")).toBeNull();
    expect(screen.getByRole("main").parentElement!.hasAttribute("inert")).toBe(false);
    // The shell's history keys still move the page beside it.
    expect(shellModalOpen()).toBe(false);
  });

  it("counts as an open modal for the shell's history keys on narrow windows", () => {
    media.narrow = true;
    useUiStore.setState({ assistantPanelOpen: true });
    mount();
    // Alt+Left or Mod+[ must not move the inert page under the sheet.
    expect(shellModalOpen()).toBe(true);
    act(pressShortcut);
    expect(shellModalOpen()).toBe(false);
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

  it("keeps a pending approval on screen when the panel is closed", () => {
    session.pendingConsent = { callId: "c1", name: "ui.journals.process" };
    const view = mount();
    expect(panel()).not.toBeNull();
    // The user closes the panel while the approval is pending.
    act(pressShortcut);
    expect(panel()).toBeNull();
    expect(screen.getByRole("alertdialog")).toBeTruthy();

    // Not even with nothing below the page, or a page that claims the column.
    for (const [bar, claimed] of [
      ["hidden", false],
      ["composer", true],
    ] as const) {
      act(() =>
        useUiStore.setState({
          assistantClosedBar: bar,
          assistantColumnClaimed: claimed,
        }),
      );
      view.rerender(
        <AssistantLayout enabled>
          <main id="app-main" className="min-h-0 flex-1 overflow-auto">
            page
          </main>
        </AssistantLayout>,
      );
      expect(panel()).toBeNull();
      expect(document.querySelector("[data-assistant-closed-bar]")).toBeNull();
      expect(screen.getByRole("alertdialog")).toBeTruthy();
    }
  });

  it("stays out of a page that claims the whole column until asked for", () => {
    useUiStore.setState({ assistantPanelOpen: true });
    const release = vi.fn();
    const claim = renderHook(
      ({ claimed }) => useAssistantColumnClaim(claimed, release),
      { initialProps: { claimed: true } },
    );
    mount();
    // The expanded table has the column: no panel beside it, no bar under it.
    expect(panel()).toBeNull();
    expect(document.querySelector("[data-assistant-closed-bar]")).toBeNull();
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);

    // A new approval opens nothing over it, but stays answerable.
    session.pendingConsent = { callId: "c1", name: "ui.journals.process" };
    act(() => useUiStore.setState({ assistantPanelOpen: false }));
    expect(panel()).toBeNull();
    expect(release).not.toHaveBeenCalled();

    // The shortcut asks for the assistant: the page gives the column back.
    act(pressShortcut);
    expect(release).toHaveBeenCalledOnce();
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
    claim.rerender({ claimed: false });
    expect(panel()).not.toBeNull();
  });

  it("leaves a claimed column to show a failed turn, but not for an approval", () => {
    const release = vi.fn();
    const claim = renderHook(
      ({ claimed }) => useAssistantColumnClaim(claimed, release),
      { initialProps: { claimed: true } },
    );
    const view = mount();
    const rerender = () =>
      view.rerender(
        <AssistantLayout enabled>
          <main id="app-main" className="min-h-0 flex-1 overflow-auto">
            page
          </main>
        </AssistantLayout>,
      );
    expect(panel()).toBeNull();

    // A tool approval is answered in the layout's dialog over the expanded
    // table: the table keeps the column.
    session.pendingConsent = { callId: "c1", name: "ui.journals.process" };
    rerender();
    expect(screen.getByRole("alertdialog")).toBeTruthy();
    expect(release).not.toHaveBeenCalled();
    expect(panel()).toBeNull();

    // A failed turn shows only in the panel, so it asks for the column the
    // way the user's own request does.
    session.pendingConsent = null;
    session.error = { code: "provider_error", message: "The provider stopped." };
    rerender();
    expect(release).toHaveBeenCalledOnce();
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
    claim.rerender({ claimed: false });
    expect(panel()).not.toBeNull();
    expect(screen.getByRole("alert").textContent).toContain("The provider stopped.");

    // Once: closing the panel again is the user's choice, and the same
    // failure does not reopen it.
    act(pressShortcut);
    expect(panel()).toBeNull();
    rerender();
    expect(panel()).toBeNull();
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
