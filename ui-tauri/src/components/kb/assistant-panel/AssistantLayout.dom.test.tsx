// @vitest-environment happy-dom
//
// Mounted: closed, a bar sits under the page in the same column; open, the
// assistant is a tab of the right panel docked beside the page (a sheet over
// it on narrow windows), which remembers its width, shares the panel with
// open details and its own surfaces (review queue, changes, logs), and offers
// a launcher once its tabs are closed.
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
const navigate = vi.hoisted(() => vi.fn());
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...props }: React.ComponentProps<"a">) => (
    <a {...props}>{children}</a>
  ),
  useNavigate: () => navigate,
}));

// The surfaces' reads, by kind; nothing reaches a daemon.
const daemon = vi.hoisted(() => ({
  data: {} as Record<string, unknown>,
  calls: [] as Array<{ kind: string; args: unknown }>,
  revert: vi.fn(async () => ({})),
}));
vi.mock("@/daemon/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/daemon/client")>()),
  useDaemon: (kind: string, args?: unknown) => {
    daemon.calls.push({ kind, args });
    const data = daemon.data[kind];
    return {
      data: data === undefined ? undefined : { kind, data },
      isLoading: false,
      isFetching: false,
      isError: false,
      refetch: vi.fn(),
    };
  },
  useDaemonInfinite: (kind: string, args?: unknown) => {
    daemon.calls.push({ kind, args });
    return {
      data: { pages: [{ kind, data: daemon.data[kind] }] },
      isLoading: false,
      isFetching: false,
      isError: false,
      hasNextPage: false,
      refetch: vi.fn(),
    };
  },
  useDaemonMutation: () => ({ mutateAsync: daemon.revert, isPending: false }),
}));
vi.mock("@/hooks/useJournalProcessingAction", () => ({
  useJournalProcessingAction: () => ({
    runJournalProcessing: vi.fn(),
    isProcessingJournals: false,
  }),
}));
// The detail's own wiring is the controller's; here it is the panel's tab.
vi.mock("@/components/transactions/dashboard/TransactionDetailController", async () => {
  const { TransactionDetailSheetFrame } = await import(
    "@/components/transactions/TransactionDetailSheetFrame"
  );
  return {
    TransactionDetailController: ({
      transaction,
      initialTab,
      onOpenChange,
    }: {
      transaction: { id: string } | null;
      initialTab: string;
      onOpenChange: (open: boolean) => void;
    }) => (
      <TransactionDetailSheetFrame
        open
        transaction={transaction as never}
        onOpenChange={onOpenChange}
      >
        {transaction ? (
          <p data-testid="kept-detail">
            {transaction.id} at {initialTab}
          </p>
        ) : null}
      </TransactionDetailSheetFrame>
    ),
  };
});
vi.mock("@/components/transactions/dashboard/model", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/transactions/dashboard/model")>()),
  toDashboardTransaction: (tx: { id: string; counterparty: string }) => ({
    id: tx.id,
    txnId: tx.id,
    counterparty: tx.counterparty,
  }),
}));

import { clearAppLogRecords, emitAppLog } from "@/lib/appLogs";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useRightPanelStore } from "@/store/rightPanel";
import {
  ASSISTANT_PANEL_DEFAULT_WIDTH,
  ASSISTANT_PANEL_MAX_WIDTH,
  DETAIL_PANEL_DEFAULT_WIDTH,
  DETAIL_PANEL_MIN_WIDTH,
  useUiStore,
} from "@/store/ui";
import { shellModalOpen } from "../shellNavigation";
import { AssistantLayout } from "./AssistantLayout";
import { useAssistantColumnClaim } from "./useAssistantColumnClaim";

function layout(enabled = true, page: React.ReactNode = "page") {
  return (
    <AssistantLayout enabled={enabled}>
      <main id="app-main" className="min-h-0 flex-1 overflow-auto">
        {page}
      </main>
    </AssistantLayout>
  );
}

function mount(enabled = true) {
  return render(layout(enabled));
}

function storedUi(): Record<string, unknown> {
  return JSON.parse(localStorage.getItem("kb.ui") ?? "{}").state ?? {};
}

function panel(): HTMLElement | null {
  return document.querySelector("[data-right-panel]");
}

/** The remembered width the docked panel asks for; CSS caps it to the row. */
function panelWidth(): string | undefined {
  return panel()?.style.width;
}

/** The sheet's remembered width; CSS caps it to the window. */
function sheetWidth(): string | undefined {
  return panel()?.style.getPropertyValue("--right-panel-width");
}

function assistantInFront(): boolean {
  return panel()?.dataset.activeTab === "assistant";
}

function pressShortcut() {
  fireEvent.keyDown(window, { key: "i", ctrlKey: true });
}

function setViewport(width: number) {
  Object.defineProperty(window, "innerWidth", { configurable: true, value: width });
  act(() => {
    window.dispatchEvent(new Event("resize"));
  });
}

/** Narrow windows get the sheet over the page instead of the docked panel. */
function useSheet(sheet: boolean) {
  Object.defineProperty(window, "matchMedia", {
    configurable: true,
    value: (query: string) => ({
      matches: sheet,
      media: query,
      addEventListener: () => {},
      removeEventListener: () => {},
    }),
  });
}

const ASSISTANT_OPEN = { assistantPanelOpen: true, rightPanelOpen: true } as const;

const QUARANTINE = {
  summary: {
    workspace: "Books",
    profile: "Main",
    count: 3,
    by_reason: [{ reason: "missing_price", count: 3 }],
    limit: 25,
    reports_blocked: true,
    attention_count: 1,
    waiting_count: 2,
    groups: [
      {
        key: "missing_price:tx-root",
        category: "missing_price",
        reason: "missing_price",
        root_transaction_id: "tx-root",
        root_occurred_at: "2025-03-01T10:00:00Z",
        root_wallet: "Cold",
        root_external_id: "abc",
        root_amount_msat: 100_000_000,
        root_direction: "inbound",
        root_asset: "BTC",
        count: 3,
        downstream_count: 2,
        blocks_reports: true,
        wallets: ["Cold"],
        earliest_occurred_at: "2025-03-01T10:00:00Z",
        evidence: {},
        actions: [],
        root_transaction_ids: ["tx-root"],
        root_count: 1,
      },
    ],
  },
  items: [],
};

const CHANGE = {
  id: "e1",
  transaction_id: "tx-9",
  transaction_external_id: "abcdef",
  source: "ai_tool",
  source_label: "Assistant",
  changed_at: new Date().toISOString(),
  summary: "Set the price",
  families: ["pricing"],
  fields: [
    {
      id: "f1",
      field: "price",
      label: "Price",
      family: "pricing",
      before_value: null,
      after_value: 61000,
      before_label: "none",
      after_label: "€ 61,000",
    },
  ],
};

async function flushMicrotasks() {
  await act(async () => {
    await Promise.resolve();
  });
}

describe("AssistantLayout", () => {
  beforeEach(() => {
    session.messages = [];
    session.pendingConsent = null;
    session.error = null;
    session.sendPrompt.mockReset();
    session.sendBlockedReason = null;
    useAssistantDraftStore.setState({ draft: "" });
    Object.defineProperty(window, "innerWidth", { configurable: true, value: 1400 });
    useSheet(false);
    useUiStore.setState({
      assistantPanelOpen: false,
      assistantPanelWidth: ASSISTANT_PANEL_DEFAULT_WIDTH,
      rightPanelOpen: false,
      rightPanelMaximized: false,
      detailPanelWidth: DETAIL_PANEL_DEFAULT_WIDTH,
      assistantClosedBar: "composer",
      assistantDockPosition: "right",
      assistantColumnClaimed: false,
    });
    useUiStore.setState({ developerToolsEnabled: false, hideSensitive: false });
    useRightPanelStore.setState({
      activeTab: "assistant",
      details: [],
      detailSlots: {},
      openedFor: null,
      surfaces: [],
      transactions: [],
    });
    daemon.data = {};
    daemon.calls = [];
    navigate.mockReset();
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

  it("docks beside the page with the shortcut and hides again", () => {
    mount();
    act(pressShortcut);
    const docked = panel()!;
    expect(docked).not.toBeNull();
    expect(docked.dataset.rightPanel).toBe("docked");
    expect(assistantInFront()).toBe(true);
    // Beside the page in the same row, not over it: no dialog, no backdrop,
    // and the page stays reachable.
    const pageColumn = screen.getByRole("main").parentElement!;
    expect(pageColumn.nextElementSibling).toBe(docked);
    expect(docked.className).not.toMatch(/\b(absolute|fixed)\b/);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(pageColumn.closest("[aria-hidden='true']")).toBeNull();
    expect(shellModalOpen()).toBe(false);
    expect(panelWidth()).toBe(`${ASSISTANT_PANEL_DEFAULT_WIDTH}px`);
    // The composer moved into the panel; the closed bar is gone.
    expect(document.querySelector("[data-assistant-closed-bar]")).toBeNull();
    expect(storedUi()).toMatchObject({ assistantPanelOpen: true, rightPanelOpen: true });

    // Hiding the panel keeps the assistant as its tab for next time.
    act(pressShortcut);
    expect(panel()).toBeNull();
    expect(storedUi()).toMatchObject({ assistantPanelOpen: true, rightPanelOpen: false });
    expect(document.querySelector("[data-assistant-closed-bar]")).not.toBeNull();
  });

  it("shows the launcher once its last tab closes, and opens surfaces by key", () => {
    useUiStore.setState(ASSISTANT_OPEN);
    render(layout(true, <input aria-label="page field" />));
    fireEvent.click(screen.getByRole("button", { name: "Close assistant panel" }));
    expect(useUiStore.getState().assistantPanelOpen).toBe(false);
    // The panel stays, offering what it can show.
    const launcher = screen.getByRole("group", { name: "Open in this panel" });
    expect(panel()!.contains(launcher)).toBe(true);
    expect(screen.queryByRole("tab")).toBeNull();
    expect(document.activeElement).toBe(launcher);
    // Details need a transaction, and say so.
    const details = screen.getByText("Transaction details").parentElement!;
    expect(details.getAttribute("aria-disabled")).toBe("true");
    expect(details.textContent).toContain("Open a transaction to see its details here.");
    fireEvent.keyDown(launcher, { key: "d" });
    expect(screen.queryByRole("tab")).toBeNull();

    // A key typed into a field is text, not a shortcut.
    const field = screen.getByRole("textbox", { name: "page field" });
    act(() => field.focus());
    fireEvent.keyDown(field, { key: "a" });
    expect(useUiStore.getState().assistantPanelOpen).toBe(false);

    act(() => launcher.focus());
    fireEvent.keyDown(launcher, { key: "a" });
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
    expect(assistantInFront()).toBe(true);
    expect(document.activeElement).toBe(panel()!.querySelector("textarea"));
  });

  it("opens the launcher from the arrow keys and Enter", () => {
    useUiStore.setState({ rightPanelOpen: true });
    mount();
    const launcher = screen.getByRole("group", { name: "Open in this panel" });
    fireEvent.keyDown(launcher, { key: "ArrowDown" });
    fireEvent.keyDown(launcher, { key: "Enter" });
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
  });

  it("adds a surface from the + menu", () => {
    useUiStore.setState({ rightPanelOpen: true });
    mount();
    fireEvent.keyDown(screen.getByRole("button", { name: "Open in side panel" }), {
      key: "Enter",
    });
    const items = screen.getAllByRole("menuitem");
    expect(items.map((item) => item.textContent)).toEqual([
      "AssistantA",
      "Review queueQ",
      "ChangesH",
      "LogsL",
      "Transaction detailsD",
    ]);
    // Changes and Logs open with developer tools, like their pages; details
    // need a transaction.
    expect(items.slice(2).map((item) => item.getAttribute("aria-disabled"))).toEqual([
      "true",
      "true",
      "true",
    ]);
    fireEvent.keyDown(items[0], { key: "a" });
    expect(screen.queryByRole("menu")).toBeNull();
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
    expect(assistantInFront()).toBe(true);
  });

  it("hides from the title-bar toggle and hands focus to it", async () => {
    useUiStore.setState(ASSISTANT_OPEN);
    render(
      layout(
        true,
        <button
          type="button"
          data-right-panel-toggle=""
          onClick={() => useUiStore.getState().toggleRightPanel()}
        >
          toggle
        </button>,
      ),
    );
    const composer = panel()!.querySelector("textarea")!;
    act(() => composer.focus());
    act(() => useUiStore.getState().toggleRightPanel());
    expect(panel()).toBeNull();
    await flushMicrotasks();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "toggle" }));

    fireEvent.click(screen.getByRole("button", { name: "toggle" }));
    expect(assistantInFront()).toBe(true);
  });

  it("maximizes over the page column and restores", () => {
    useUiStore.setState(ASSISTANT_OPEN);
    mount();
    const pageColumn = screen.getByRole("main").parentElement!;
    fireEvent.click(screen.getByRole("button", { name: "Maximize panel" }));
    expect(panel()!.hasAttribute("data-maximized")).toBe(true);
    expect(panel()!.style.width).toBe("");
    expect(screen.queryByRole("separator")).toBeNull();
    // The page stays mounted, out of view and out of reach.
    expect(pageColumn.className).toMatch(/\bhidden\b/);
    expect(pageColumn.hasAttribute("inert")).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Restore panel size" }));
    expect(pageColumn.className).not.toMatch(/\bhidden\b/);
    expect(panelWidth()).toBe(`${ASSISTANT_PANEL_DEFAULT_WIDTH}px`);

    // Hiding the panel drops the maximized state with it.
    fireEvent.click(screen.getByRole("button", { name: "Maximize panel" }));
    act(pressShortcut);
    expect(useUiStore.getState().rightPanelMaximized).toBe(false);
  });

  it("resizes from its left edge and remembers the width", () => {
    useUiStore.setState(ASSISTANT_OPEN);
    const view = mount();
    const handle = screen.getByRole("separator", { name: "Resize side panel" });

    fireEvent.pointerDown(handle, { button: 0, clientX: 900, pointerId: 1 });
    fireEvent.pointerMove(handle, { clientX: 800, pointerId: 1 });
    const dragged = ASSISTANT_PANEL_DEFAULT_WIDTH + 100;
    expect(panelWidth()).toBe(`${dragged}px`);
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
    expect(panelWidth()).toBe(`${ASSISTANT_PANEL_MAX_WIDTH - 16}px`);
  });

  it("keeps the page its minimum beside the docked panel", () => {
    useUiStore.setState({ ...ASSISTANT_OPEN, assistantPanelWidth: ASSISTANT_PANEL_MAX_WIDTH });
    const view = mount();
    // A 1000px row leaves 520px for the panel beside a 480px page.
    const row = panel()!.parentElement!;
    Object.defineProperty(row, "clientWidth", { configurable: true, value: 1000 });
    panel()!.getBoundingClientRect = () =>
      ({ width: 520, height: 800, top: 0, left: 480, right: 1000, bottom: 800, x: 480, y: 0 }) as DOMRect;
    expect(panel()!.style.maxWidth).toContain("100% - 480px");
    view.rerender(layout());
    const handle = screen.getByRole("separator", { name: "Resize side panel" });
    expect(handle.getAttribute("aria-valuemax")).toBe("520");
    expect(handle.getAttribute("aria-valuenow")).toBe("520");
    // The keyboard steps from the rendered width.
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(useUiStore.getState().assistantPanelWidth).toBe(520 - 16);
  });

  it("gives the page its cursor and selection back when it closes mid-resize", () => {
    useUiStore.setState(ASSISTANT_OPEN);
    mount();
    const handle = screen.getByRole("separator", { name: "Resize side panel" });
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

  it("is a sheet over the page on a narrow window: focus moves in, Escape hides it, focus returns", async () => {
    useSheet(true);
    render(layout(true, <button type="button">page action</button>));
    const pageAction = screen.getByRole("button", { name: "page action" });
    act(() => pageAction.focus());
    act(pressShortcut);

    const sheet = screen.getByRole("dialog");
    expect(sheet).toBe(panel());
    expect(sheet.dataset.rightPanel).toBe("sheet");
    expect(screen.getByRole("dialog", { name: "Side panel: Assistant" })).toBe(sheet);
    expect(sheetWidth()).toBe(`${ASSISTANT_PANEL_DEFAULT_WIDTH}px`);
    // The covered page is hidden from assistive tech, and focus is in the
    // panel's composer.
    expect(
      screen.getByRole("main", { hidden: true }).closest("[aria-hidden='true']"),
    ).not.toBeNull();
    expect(document.activeElement).toBe(sheet.querySelector("textarea"));
    // The shell's history keys leave the page under it alone.
    expect(shellModalOpen()).toBe(true);
    // Nothing to maximize over: the sheet closes instead.
    expect(screen.queryByRole("button", { name: "Maximize panel" })).toBeNull();

    act(() => {
      fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    });
    expect(panel()).toBeNull();
    expect(shellModalOpen()).toBe(false);
    expect(useUiStore.getState().rightPanelOpen).toBe(false);
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
    // The dialog's focus scope hands focus back on a timer after unmounting.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(document.activeElement).toBe(pageAction);
  });

  it("reports the sheet's width it renders at when the window narrows", () => {
    useSheet(true);
    useUiStore.setState({ ...ASSISTANT_OPEN, assistantPanelWidth: ASSISTANT_PANEL_MAX_WIDTH });
    mount();
    const handle = screen.getByRole("separator", { name: "Resize side panel" });
    expect(handle.getAttribute("aria-valuenow")).toBe(String(ASSISTANT_PANEL_MAX_WIDTH));

    // A 568px window leaves the page its 48px strip, so CSS caps the
    // remembered 640px at 520px; the separator follows.
    setViewport(568);
    expect(handle.getAttribute("aria-valuemax")).toBe("520");
    expect(handle.getAttribute("aria-valuenow")).toBe("520");
  });

  it("shares the panel with an open detail as tabs", () => {
    const closeDetail = vi.fn(() => {
      act(() => {
        useRightPanelStore.getState().hideDetail(key);
      });
    });
    useUiStore.setState(ASSISTANT_OPEN);
    mount();
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Assistant",
    ]);

    let key = 0;
    act(() => {
      key = useRightPanelStore
        .getState()
        .showDetail({ label: "Details", close: closeDetail, transactionId: null, owner: null });
    });
    // The new detail comes to the front; the assistant stays a tab.
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual(["Assistant", "Details"]);
    expect(panel()!.dataset.activeTab).toBe(`detail:${key}`);
    expect(tabs[1].getAttribute("aria-selected")).toBe("true");
    // The details keep their own, wider width.
    expect(panelWidth()).toBe(`${DETAIL_PANEL_DEFAULT_WIDTH}px`);
    expect(
      screen.getByRole("separator", { name: "Resize side panel" }).getAttribute("aria-valuemin"),
    ).toBe(String(DETAIL_PANEL_MIN_WIDTH));

    // The shortcut brings the assistant forward rather than hiding it.
    act(pressShortcut);
    expect(assistantInFront()).toBe(true);
    expect(useUiStore.getState().rightPanelOpen).toBe(true);
    expect(panelWidth()).toBe(`${ASSISTANT_PANEL_DEFAULT_WIDTH}px`);
    expect(document.activeElement).toBe(panel()!.querySelector("textarea"));

    // A tab's close button closes only that tab; Delete does the same.
    fireEvent.click(screen.getByRole("tab", { name: "Details" }));
    expect(panel()!.dataset.activeTab).toBe(`detail:${key}`);
    fireEvent.keyDown(screen.getByRole("tab", { name: "Details" }), { key: "Delete" });
    expect(closeDetail).toHaveBeenCalledOnce();
    expect(assistantInFront()).toBe(true);
    expect(screen.getAllByRole("tab")).toHaveLength(1);
  });

  it("closes the panel again with the detail that opened it", () => {
    let key = 0;
    const closeDetail = vi.fn(() => {
      act(() => useRightPanelStore.getState().hideDetail(key));
    });
    mount();
    act(() => {
      key = useRightPanelStore.getState().showDetail({ label: "Details", close: closeDetail, transactionId: null, owner: null });
    });
    expect(useUiStore.getState().rightPanelOpen).toBe(true);
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual(["Details"]);
    fireEvent.click(screen.getByRole("button", { name: "Close Details" }));
    expect(closeDetail).toHaveBeenCalledOnce();
    expect(panel()).toBeNull();
    expect(useUiStore.getState().rightPanelOpen).toBe(false);
  });

  it("closes an open detail when the panel hides, keeping the assistant", () => {
    const closeDetail = vi.fn();
    useUiStore.setState(ASSISTANT_OPEN);
    mount();
    act(() => {
      useRightPanelStore.getState().showDetail({ label: "Details", close: closeDetail, transactionId: null, owner: null });
    });
    act(() => useUiStore.getState().toggleRightPanel());
    expect(closeDetail).toHaveBeenCalledOnce();
    expect(useUiStore.getState().assistantPanelOpen).toBe(true);
  });

  it("reads nothing until the review queue opens, then opens a cause in a tab of its own", () => {
    daemon.data["ui.journals.quarantine"] = QUARANTINE;
    daemon.data["ui.review.badges"] = {
      quarantine: 3,
      quarantine_attention: 1,
      journals_needs_processing: false,
      swaps: 2,
    };
    daemon.data["ui.transactions.resolve"] = {
      transaction: { id: "tx-root", counterparty: "Bitstamp" },
    };
    useUiStore.setState({ rightPanelOpen: true });
    mount();
    const launcher = screen.getByRole("group", { name: "Open in this panel" });
    expect(daemon.calls).toEqual([]);

    fireEvent.keyDown(launcher, { key: "q" });
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual(["Review queue"]);
    expect(panel()!.dataset.activeTab).toBe("review");
    const queue = screen.getByRole("region", { name: "Review queue" });
    expect(queue.textContent).toContain("Reports are blocked until these are resolved.");
    expect(queue.textContent).toContain("1 cause needs you · 2 waiting");
    expect(queue.textContent).toContain("2 transfers to match");
    fireEvent.click(screen.getByRole("button", { name: "2 transfers to match" }));
    expect(navigate).toHaveBeenCalledWith({ to: "/swaps" });

    // The cause opens its root in a kept tab, at the evidence's tab, and
    // closing that tab returns to the queue.
    const cause = screen.getByText("Cold", { exact: false }).closest("button")!;
    fireEvent.click(cause);
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Review queue",
      "Bitstamp",
    ]);
    expect(screen.getByTestId("kept-detail").textContent).toBe("tx-root at pricing");
    expect(screen.getByRole("tab", { name: "Bitstamp" }).querySelector(".italic")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Close Bitstamp" }));
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual(["Review queue"]);
    expect(panel()!.dataset.activeTab).toBe("review");
    expect(useRightPanelStore.getState().transactions).toEqual([]);
  });

  it("comes back from the title-bar toggle with the tab that was in front", () => {
    daemon.data["ui.journals.quarantine"] = QUARANTINE;
    useUiStore.setState(ASSISTANT_OPEN);
    mount();
    act(() => useRightPanelStore.getState().openSurface("review"));
    expect(panel()!.dataset.activeTab).toBe("review");
    act(() => useUiStore.getState().toggleRightPanel());
    expect(panel()).toBeNull();
    act(() => useUiStore.getState().toggleRightPanel());
    expect(panel()!.dataset.activeTab).toBe("review");

    // Closing the surface in front brings the assistant forward, and the
    // shortcut then hides the panel rather than "bringing it forward" again.
    fireEvent.click(screen.getByRole("button", { name: "Close Review queue" }));
    expect(useRightPanelStore.getState().activeTab).toBe("assistant");
    act(pressShortcut);
    expect(panel()).toBeNull();
  });

  it("keeps a page's preview open in a tab of its own when pinned", () => {
    daemon.data["ui.transactions.resolve"] = {
      transaction: { id: "tx-1", counterparty: "Bitstamp" },
    };
    useUiStore.setState(ASSISTANT_OPEN);
    mount();
    let key = 0;
    const closePreview = vi.fn(() => {
      act(() => useRightPanelStore.getState().hideDetail(key));
    });
    act(() => {
      key = useRightPanelStore
        .getState()
        .showDetail({ label: "Kraken", close: closePreview, transactionId: "tx-1", owner: null });
    });
    // A page's preview reads in italics: its next transaction takes the tab.
    expect(screen.getByRole("tab", { name: "Kraken" }).querySelector(".italic")).not.toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Keep open" }));
    expect(closePreview).toHaveBeenCalledOnce();
    expect(useUiStore.getState().rightPanelOpen).toBe(true);
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Assistant",
      "Bitstamp",
    ]);
    expect(screen.queryByRole("button", { name: "Keep open" })).toBeNull();

    // The page's next transaction opens beside it instead of replacing it.
    act(() => {
      useRightPanelStore
        .getState()
        .showDetail({ label: "Kraken", close: vi.fn(), transactionId: "tx-2", owner: null });
    });
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "Assistant",
      "Bitstamp",
      "Kraken",
    ]);
  });

  it("shows recent changes as a diff while developer tools are on", () => {
    daemon.data["ui.activity.history"] = { events: [CHANGE], has_more: false, limit: 30 };
    useUiStore.setState({ rightPanelOpen: true });
    mount();
    const launcher = screen.getByRole("group", { name: "Open in this panel" });
    // Locked like the Activity page: the row says why, its key does nothing.
    const locked = screen.getByText("Changes").parentElement!;
    expect(locked.getAttribute("aria-disabled")).toBe("true");
    expect(locked.textContent).toContain("Turn on developer tools to open this here.");
    fireEvent.keyDown(launcher, { key: "h" });
    expect(screen.queryByRole("tab")).toBeNull();

    act(() => useUiStore.setState({ developerToolsEnabled: true }));
    fireEvent.keyDown(screen.getByRole("group", { name: "Open in this panel" }), { key: "h" });
    expect(panel()!.dataset.activeTab).toBe("changes");
    const toggle = screen.getByRole("button", { name: /Set the price/ });
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(toggle);
    const diff = screen.getByRole("region", { name: "Changes" }).textContent;
    expect(diff).toContain("Before: none");
    expect(diff).toContain("After: € 61,000");

    fireEvent.click(screen.getByRole("button", { name: "abcdef" }));
    expect(useRightPanelStore.getState().transactions.map((entry) => entry.transactionId)).toEqual([
      "tx-9",
    ]);

    // Turning developer tools off takes the surface with it.
    act(() => useUiStore.setState({ developerToolsEnabled: false }));
    expect(useRightPanelStore.getState().surfaces).toEqual([]);
    expect(screen.queryByRole("tab", { name: "Changes" })).toBeNull();
  });

  it("tails the in-memory log, filtered by level", () => {
    clearAppLogRecords();
    emitAppLog({ level: "warning", module: "sync", file: "sync.ts", line: 1, msg: "wallet sync slow", fields: {} });
    useUiStore.setState({ rightPanelOpen: true, developerToolsEnabled: true });
    mount();
    fireEvent.keyDown(screen.getByRole("group", { name: "Open in this panel" }), { key: "l" });
    expect(screen.getByRole("log", { name: "Logs" }).textContent).toContain("wallet sync slow");
    act(() => {
      emitAppLog({ level: "error", module: "rpc", file: "rpc.ts", line: 2, msg: "backend refused", fields: {} });
    });
    expect(screen.getByRole("log").textContent).toContain("backend refused");

    fireEvent.click(screen.getByRole("radio", { name: "Errors" }));
    expect(screen.getByRole("log").textContent).not.toContain("wallet sync slow");
    fireEvent.change(screen.getByRole("searchbox", { name: "Filter lines" }), {
      target: { value: "nothing like it" },
    });
    expect(screen.getByText("No lines match")).toBeTruthy();
    clearAppLogRecords();
  });

  it("opens from the launcher and focuses the panel composer", () => {
    useUiStore.setState({ assistantClosedBar: "launcher" });
    mount();
    expect(screen.queryByRole("textbox")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Open assistant panel" }));
    expect(panel()).not.toBeNull();
    expect(document.activeElement).toBe(screen.getByRole("textbox"));
  });

  it("keeps typing in a composer when a send from the closed bar opens the panel", () => {
    // Right after launch: no focus request has been made yet.
    useUiStore.setState({ assistantPanelFocusRequest: 0 });
    useAssistantDraftStore.setState({ draft: "Why 11 pending?" });
    mount();
    const closedComposer = screen.getByRole("textbox");
    act(() => closedComposer.focus());
    fireEvent.keyDown(closedComposer, { key: "Enter" });

    expect(session.sendPrompt).toHaveBeenLastCalledWith("Why 11 pending?");
    // The bar's composer is gone with the bar; the panel's has the focus, so
    // a follow-up can be typed straight away.
    const sheet = panel();
    expect(sheet).not.toBeNull();
    expect(document.activeElement).toBe(sheet!.querySelector("textarea"));

    // A send from the open panel leaves focus where it is.
    const requests = useUiStore.getState().assistantPanelFocusRequest;
    act(() => useAssistantDraftStore.setState({ draft: "And failed?" }));
    fireEvent.keyDown(sheet!.querySelector("textarea")!, { key: "Enter" });
    expect(session.sendPrompt).toHaveBeenLastCalledWith("And failed?");
    expect(useUiStore.getState().assistantPanelFocusRequest).toBe(requests);
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

    act(() => useUiStore.setState(ASSISTANT_OPEN));
    view.rerender(layout());
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

  // The consent dialog is mocked inline, so an open sheet would hide it from
  // role queries; the real one is its own layer above the panel.
  const consentDialog = () => document.querySelector("[role='alertdialog']");

  it("opens itself for a consent prompt", () => {
    session.pendingConsent = { callId: "c1", name: "ui.journals.process" };
    mount();
    expect(panel()).not.toBeNull();
    expect(consentDialog()).not.toBeNull();
  });

  it("keeps a pending approval on screen when the panel is closed", () => {
    session.pendingConsent = { callId: "c1", name: "ui.journals.process" };
    const view = mount();
    expect(panel()).not.toBeNull();
    // The user closes the panel while the approval is pending.
    act(pressShortcut);
    expect(panel()).toBeNull();
    expect(consentDialog()).not.toBeNull();

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
      view.rerender(layout());
      expect(panel()).toBeNull();
      expect(document.querySelector("[data-assistant-closed-bar]")).toBeNull();
      expect(consentDialog()).not.toBeNull();
    }
  });

  it("stays out of a page that claims the whole column until asked for", () => {
    useUiStore.setState(ASSISTANT_OPEN);
    const release = vi.fn();
    const claim = renderHook(
      ({ claimed }) => useAssistantColumnClaim(claimed, release),
      { initialProps: { claimed: true } },
    );
    mount();
    // The expanded table has the column: no panel over it, no bar under it.
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
    const rerender = () => view.rerender(layout());
    expect(panel()).toBeNull();

    // A tool approval is answered in the layout's dialog over the expanded
    // table: the table keeps the column.
    session.pendingConsent = { callId: "c1", name: "ui.journals.process" };
    rerender();
    expect(consentDialog()).not.toBeNull();
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
    useUiStore.setState(ASSISTANT_OPEN);
    mount(false);
    expect(panel()).toBeNull();
    expect(document.querySelector("[data-assistant-closed-bar]")).toBeNull();
    act(pressShortcut);
    expect(panel()).toBeNull();
  });
});
