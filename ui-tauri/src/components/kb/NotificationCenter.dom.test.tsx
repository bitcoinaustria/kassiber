// @vitest-environment happy-dom
//
// Mounted, not static: the lanes, actions, and details only exist once the
// popover is open, and what reaches `kb.ui` depends on the live store.
import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const storage = vi.hoisted(() => {
  const map = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => void map.set(key, value),
    removeItem: (key: string) => void map.delete(key),
  });
  // Radix positions the popover with a ResizeObserver this DOM does not ship.
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
  return map;
});

const navigate = vi.hoisted(() => vi.fn());
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));

import i18n from "@/i18n";
import {
  conditionKey,
  MAX_RESULTS,
  resetNotificationStore,
  useNotificationStore,
  type ConditionInput,
  type NotificationOperation,
} from "@/store/notifications";
import type { ActiveBook } from "@/lib/activeBook";
import { UI_STATE_VERSION, useUiStore } from "@/store/ui";

import { useAssistantBridge } from "@/components/ai/assistantBridge";
import { NotificationCenter } from "./NotificationCenter";
import { resolveEntryAction } from "./notificationRouting";
import { requestSettingsAnchor } from "./settings/settingsSearch";

vi.mock("./settings/settingsSearch", async (original) => ({
  ...(await original<typeof import("./settings/settingsSearch")>()),
  requestSettingsAnchor: vi.fn(),
}));

const MINUTE = 60_000;

// Two books of one project: they share a database, so only the daemon ids
// tell them apart.
const ACTIVE_A: ActiveBook = {
  key: '["db:/p.sqlite3","ws","pf-a"]',
  expected: { workspace_id: "ws", profile_id: "pf-a" },
};
const ACTIVE_B: ActiveBook = {
  key: '["db:/p.sqlite3","ws","pf-b"]',
  expected: { workspace_id: "ws", profile_id: "pf-b" },
};
const A = ACTIVE_A.key;
/** The book the shell reports open; tests switch it. */
let currentBook: ActiveBook | null = ACTIVE_A;

// Strings that must never surface outside Normal, or reach disk at all.
const SENTINEL_LABEL = "Sentinel Cold Vault";
const SENTINEL_PATH = "/Users/sentinel/secret-wallet.json";
const SENTINEL_AMOUNT = "0.4242 BTC";

function sourceFailure(source: string, label: string, message: string): ConditionInput {
  return {
    code: "source.refresh-failed",
    scope: { book: A, source },
    severity: "error",
    priority: "attention",
    params: { sources: [label] },
    action: { type: "navigate", to: "/connections" },
    details: [{ label, message, phraseKey: "failed" }],
  };
}

function refreshFailure(book: string | null, forceFull = false): ConditionInput {
  return {
    code: "book-refresh.failed",
    scope: { book, expected: book === A ? ACTIVE_A.expected : undefined },
    severity: "error",
    priority: "attention",
    params: { forceFull },
    action: {
      type: "operation",
      operation: { kind: "refresh-book", forceFull },
      review: "/connections",
    },
  };
}

function openBell(props: Partial<React.ComponentProps<typeof NotificationCenter>> = {}) {
  const runOperation =
    vi.fn<React.ComponentProps<typeof NotificationCenter>["runOperation"]>();
  render(
    <NotificationCenter
      currentBook={currentBook}
      resolveCurrentBook={() => currentBook}
      runOperation={runOperation}
      {...props}
    />,
  );
  act(() => {
    fireEvent.click(
      screen.getByRole("button", { name: /^(Notifications|Benachrichtigungen)/ }),
    );
  });
  return { runOperation };
}

function openMoreActions(row: HTMLElement) {
  const trigger = within(row).getByRole("button", { name: /^More actions/ });
  act(() => {
    fireEvent.pointerDown(trigger, { button: 0, ctrlKey: false, pointerType: "mouse" });
  });
  if (trigger.getAttribute("aria-expanded") !== "true") {
    act(() => void fireEvent.keyDown(trigger, { key: "Enter" }));
  }
}

function reopen() {
  act(() => {
    fireEvent.click(
      screen.getByRole("button", { name: /^(Notifications|Benachrichtigungen)/ }),
    );
  });
}

function entry(title: string): HTMLElement {
  const match = [
    ...screen.queryAllByTestId("notification-entry"),
    ...screen.queryAllByTestId("notification-attention"),
    ...screen.queryAllByTestId("notification-running"),
  ].find((element) => element.textContent?.includes(title));
  if (!match) throw new Error(`no entry titled ${title}`);
  return match;
}

function section(id: string): HTMLElement | null {
  return document.querySelector(`[data-section="${id}"]`);
}

/** Visible text plus every aria-label and title: what assistive tech reads. */
function renderedText(): string {
  const parts = [document.body.textContent ?? ""];
  for (const element of document.querySelectorAll("[aria-label],[title]")) {
    parts.push(element.getAttribute("aria-label") ?? "");
    parts.push(element.getAttribute("title") ?? "");
  }
  return parts.join("\n");
}

beforeEach(async () => {
  navigate.mockReset();
  currentBook = ACTIVE_A;
  storage.clear();
  resetNotificationStore();
  await i18n.changeLanguage("en");
  useUiStore.setState({
    lang: "en",
    developerToolsEnabled: false,
    privacyMode: "normal",
  });
});

afterEach(() => {
  cleanup();
});

describe("NotificationCenter lanes", () => {
  it("orders Needs attention, Running, then Recent, blockers first", () => {
    const store = useNotificationStore.getState();
    store.raiseCondition(sourceFailure("s1", "Hot", "timeout"));
    store.raiseCondition({
      ...sourceFailure("s2", "Cold", "refused"),
      code: "book-refresh.backend-required",
      scope: { book: A },
      priority: "blocking",
    });
    store.reconcileConditions({
      present: [
        {
          code: "journals.stale",
          scope: { book: A },
          severity: "warning",
          priority: "attention",
          action: {
            type: "operation",
            operation: { kind: "process-journals", autoPair: true },
            review: "/journals",
          },
        },
      ],
    });
    store.startJob({
      id: "book-refresh",
      code: "book-refresh",
      scope: { book: A },
      progress: { value: 40 },
    });
    store.addResult({
      code: "journals.processed",
      scope: { book: A },
      severity: "info",
      params: { transactions: 3, entries: 5 },
    });
    store.addResult({
      code: "report.exported",
      scope: { book: A },
      severity: "info",
      action: { type: "navigate", to: "/reports" },
    });
    openBell();

    const order = [...document.querySelectorAll("[data-section]")].map((el) =>
      el.getAttribute("data-section"),
    );
    expect(order).toEqual(["attention", "running", "recent"]);
    const attention = within(section("attention")!).getAllByTestId(
      "notification-attention",
    );
    // A blocker leads; then the live journal state; then the rest.
    expect(attention[0].textContent).toContain("Add a backend to sync wallets");
    expect(attention[1].textContent).toContain("Journals need processing");
    expect(attention[2].textContent).toContain("1 source could not refresh");
    expect(within(section("running")!).getByText("Refreshing book")).toBeTruthy();
    expect(within(section("recent")!).getByText("Journals processed")).toBeTruthy();
    expect(within(section("recent")!).getByText("Report exported")).toBeTruthy();
  });

  it("badges unresolved attention groups only", () => {
    const store = useNotificationStore.getState();
    store.raiseCondition(sourceFailure("s1", "Hot", "timeout"));
    store.raiseCondition(sourceFailure("s2", "Cold", "refused"));
    store.raiseCondition(refreshFailure(A));
    for (let index = 0; index < 5; index += 1) {
      store.addResult({
        code: "book-refresh.finished",
        scope: { book: `other-${index}` },
        severity: "info",
      });
    }
    render(<NotificationCenter currentBook={currentBook} runOperation={vi.fn()} />);
    // Two sources fold into one group; results never count.
    expect(
      screen.getByRole("button", { name: "Notifications (2 active)" }),
    ).toBeTruthy();
  });

  it("keeps every condition while more than 30 receipts arrive", () => {
    const store = useNotificationStore.getState();
    store.raiseCondition(refreshFailure(A));
    for (let index = 0; index < 45; index += 1) {
      store.addResult({
        code: "journals.processed",
        scope: { book: `book-${index}` },
        severity: "info",
        params: { transactions: index },
      });
    }
    expect(useNotificationStore.getState().results).toHaveLength(MAX_RESULTS);
    expect(
      useNotificationStore.getState().conditions[
        conditionKey("book-refresh.failed", { book: A })
      ],
    ).toBeDefined();

    openBell();
    expect(entry("Book refresh failed")).toBeTruthy();
    expect(
      within(section("recent")!).getAllByTestId("notification-entry"),
    ).toHaveLength(MAX_RESULTS);
  });

  it("Clear recent removes results, never conditions or jobs", () => {
    const store = useNotificationStore.getState();
    store.raiseCondition(refreshFailure(A));
    store.startJob({
      id: "journal-processing",
      code: "journal-processing",
      scope: { book: A },
      progress: { indeterminate: true },
    });
    store.addResult({ code: "book-refresh.finished", scope: { book: A }, severity: "info" });
    openBell();

    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Clear recent" }));
    });
    expect(useNotificationStore.getState().results).toEqual([]);
    expect(section("recent")).toBeNull();
    expect(entry("Book refresh failed")).toBeTruthy();
    expect(entry("Processing journals")).toBeTruthy();
  });

  it("offers no dismiss on a condition, and dismisses one result", () => {
    useNotificationStore.getState().raiseCondition(refreshFailure(A));
    useNotificationStore.getState().addResult({
      code: "book-refresh.finished",
      scope: { book: A },
      severity: "info",
    });
    openBell();
    expect(
      within(entry("Book refresh failed")).queryByRole("button", { name: /Dismiss/ }),
    ).toBeNull();
    act(() => {
      fireEvent.click(
        screen.getByRole("button", { name: "Dismiss notification: Book refreshed" }),
      );
    });
    expect(useNotificationStore.getState().results).toEqual([]);
  });

  it("marks entries seen on open without resolving them", () => {
    useNotificationStore.getState().raiseCondition(refreshFailure(A));
    openBell();
    const [condition] = Object.values(useNotificationStore.getState().conditions);
    expect(condition.seenAt).toBeTruthy();
    expect(condition.occurrenceCount).toBe(1);
  });

  it("shows how often a condition occurred", () => {
    const store = useNotificationStore.getState();
    store.raiseCondition(refreshFailure(A));
    store.raiseCondition(refreshFailure(A));
    store.raiseCondition(refreshFailure(A));
    openBell();
    const row = entry("Book refresh failed");
    expect(within(row).getByText("×3")).toBeTruthy();
    expect(within(row).getByText("Arrived 3 times")).toBeTruthy();
  });
});

describe("NotificationCenter actions and scope", () => {
  it("retries with the original options for the open book", () => {
    useNotificationStore.getState().raiseCondition(refreshFailure(A, true));
    const { runOperation } = openBell();
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Retry rescan" }));
    });
    expect(runOperation).toHaveBeenCalledWith(
      { kind: "refresh-book", forceFull: true },
      ACTIVE_A.expected,
    );
    expect(navigate).not.toHaveBeenCalled();
  });

  it("disables an operation already running", () => {
    useNotificationStore.getState().raiseCondition(refreshFailure(A));
    openBell({ pendingOperations: { "refresh-book": true } });
    expect(
      (screen.getByRole("button", { name: "Refreshing..." }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("never replays a stale action on another book", () => {
    // A late failure for book A lands while book B of the same project is
    // open.
    currentBook = ACTIVE_B;
    useNotificationStore.getState().raiseCondition(refreshFailure(A));
    useNotificationStore.getState().addResult({
      code: "book-refresh.pending",
      scope: { book: A, expected: ACTIVE_A.expected },
      severity: "info",
      action: {
        type: "operation",
        operation: { kind: "refresh-book", forceFull: false },
        review: "/connections",
      },
    });
    const { runOperation } = openBell();
    // A's condition is not B's to show or to count; it is kept, not evicted.
    expect(screen.queryByText("Book refresh failed")).toBeNull();
    expect(screen.getByRole("button", { name: "Notifications" })).toBeTruthy();
    const row = entry("Some sources are still queued");
    expect(within(row).getByText("Started in another book")).toBeTruthy();
    act(() => {
      fireEvent.click(within(row).getByRole("button", { name: "Review" }));
    });
    expect(runOperation).not.toHaveBeenCalled();
    expect(navigate).toHaveBeenCalledWith({ to: "/connections" });

    // Back on A, its condition is there again with a live Retry.
    cleanup();
    currentBook = ACTIVE_A;
    const second = openBell();
    act(() => {
      fireEvent.click(
        within(entry("Book refresh failed")).getByRole("button", { name: "Retry" }),
      );
    });
    expect(second.runOperation).toHaveBeenCalledWith(
      { kind: "refresh-book", forceFull: false },
      ACTIVE_A.expected,
    );
  });

  it("revalidates scope at click time, not render time", () => {
    useNotificationStore.getState().addResult({
      code: "book-refresh.pending",
      scope: { book: A, expected: ACTIVE_A.expected },
      severity: "info",
      action: {
        type: "operation",
        operation: { kind: "refresh-book", forceFull: false },
        review: "/connections",
      },
    });
    const { runOperation } = openBell();
    const button = within(entry("Some sources are still queued")).getByRole("button", {
      name: "Retry",
    });
    // The book changes under an open list, before the click lands.
    currentBook = ACTIVE_B;
    act(() => void fireEvent.click(button));
    expect(runOperation).not.toHaveBeenCalled();
    expect(navigate).toHaveBeenCalledWith({ to: "/connections" });
  });

  it("refuses an operation while a book switch is in flight", () => {
    useNotificationStore.getState().raiseCondition(refreshFailure(A));
    const { runOperation } = openBell();
    const button = within(entry("Book refresh failed")).getByRole("button", {
      name: "Retry",
    });
    // A switch has started: the shell knows no open book until it lands.
    currentBook = null;
    act(() => void fireEvent.click(button));
    expect(runOperation).not.toHaveBeenCalled();
    expect(navigate).toHaveBeenCalledWith({ to: "/connections" });
  });

  it("never replays an operation that carries no daemon ids", () => {
    useNotificationStore.getState().raiseCondition({
      ...refreshFailure(A),
      scope: { book: A },
    });
    const { runOperation } = openBell();
    act(() => {
      fireEvent.click(
        within(entry("Book refresh failed")).getByRole("button", { name: "Review" }),
      );
    });
    expect(runOperation).not.toHaveBeenCalled();
    expect(navigate).toHaveBeenCalledWith({ to: "/connections" });
  });

  it("gives the same destinations in English and Austrian German", async () => {
    const store = useNotificationStore.getState();
    store.raiseCondition(sourceFailure("s1", "Hot", "timeout"));
    store.raiseCondition({
      code: "book-refresh.backend-required",
      scope: { book: A },
      severity: "warning",
      priority: "attention",
      action: { type: "navigate", to: "/settings/bitcoin" },
    });
    const clickAll = () => {
      openBell();
      const rows = screen.getAllByTestId("notification-attention");
      for (let index = 0; index < rows.length; index += 1) {
        const current = screen.getAllByTestId("notification-attention")[index];
        act(() => void fireEvent.click(within(current).getAllByRole("button")[0]));
        reopen();
      }
      cleanup();
      return navigate.mock.calls.map(([call]) => call);
    };
    const english = clickAll();
    navigate.mockReset();
    await act(async () => {
      await i18n.changeLanguage("de");
    });
    useUiStore.setState({ lang: "de" });
    const german = clickAll();
    expect(english).toEqual([
      { to: "/settings/bitcoin" },
      { to: "/connections" },
    ]);
    expect(german).toEqual(english);
  });

  it("fails closed on unknown routes and operations", () => {
    const store = useNotificationStore.getState();
    store.addResult({
      code: "book-refresh.pending",
      scope: { book: A },
      severity: "info",
      action: { type: "navigate", to: "https://example.com" },
    });
    store.addResult({
      code: "journals.processed",
      scope: { book: A },
      severity: "info",
      action: {
        type: "operation",
        operation: { kind: "delete-book" } as unknown as NotificationOperation,
        review: "/journals",
      },
    });
    store.addResult({
      code: "connection.synced",
      scope: { book: A, source: "connection:x" },
      severity: "info",
      action: { type: "open-connection", connectionId: "" },
    });
    openBell();
    for (const title of [
      "Some sources are still queued",
      "Journals processed",
      "Wallet synced",
    ]) {
      expect(entry(title)).toBeTruthy();
    }
    expect(screen.queryByRole("button", { name: "Open wallets" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Open journals" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Open connection" })).toBeNull();
    expect(
      resolveEntryAction(
        { type: "navigate", to: "/nope" },
        { book: A },
        ACTIVE_A,
        false,
      ),
    ).toBeNull();
  });
});

describe("NotificationCenter entries", () => {
  it("shows when each entry arrived, relative and in full", () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(Date.now() - 2 * MINUTE - 5_000);
    useNotificationStore.getState().addResult({
      code: "report.exported",
      scope: { book: A },
      severity: "info",
    });
    vi.useRealTimers();
    openBell();
    const time = within(entry("Report exported")).getByText("2 min. ago");
    expect(time.tagName).toBe("TIME");
    expect(time.getAttribute("dateTime")).toBe(
      useNotificationStore.getState().results[0].lastOccurredAt,
    );
    expect(time.getAttribute("title")).toMatch(/\d{4}/);
  });

  it("marks unseen new transactions with a dot until the center opens", () => {
    useNotificationStore.getState().addResult({
      code: "activity.new",
      scope: { book: A, expected: ACTIVE_A.expected },
      severity: "info",
      params: { transactions: 2, incoming: 2, outgoing: 0, transactionIds: ["t1", "t2"] },
      action: { type: "navigate", to: "/transactions" },
    });
    render(
      <NotificationCenter
        currentBook={currentBook}
        resolveCurrentBook={() => currentBook}
        runOperation={vi.fn()}
      />,
    );
    expect(screen.getByTestId("notification-activity-dot")).toBeTruthy();
    const bell = screen.getByRole("button", { name: "Notifications: new transactions" });
    act(() => void fireEvent.click(bell));
    const row = entry("2 new transactions");
    expect(within(row).getByText("2 incoming")).toBeTruthy();
    expect(within(row).getByRole("button", { name: "Open transactions" })).toBeTruthy();
    expect(screen.queryByTestId("notification-activity-dot")).toBeNull();
  });

  it("offers the assistant from More actions, without raw server text", () => {
    const sendPrompt = vi.fn(() => true);
    useAssistantBridge.setState({
      assistant: {
        sendPrompt,
        captureBook: () => ({
          daemonSession: 1,
          scope: { workspace_id: "ws", profile_id: "pf-a" },
        }),
      } as never,
    });
    try {
      useNotificationStore.getState().raiseCondition(
        sourceFailure("s1", "Cold storage", "connection refused (10.0.0.5:50002)"),
      );
      openBell();
      const row = entry("1 source could not refresh");
      expect(within(row).queryByText(/Ask the assistant/)).toBeNull();
      openMoreActions(row);
      act(() => {
        fireEvent.click(screen.getByRole("menuitem", { name: /Ask the assistant/ }));
      });
      expect(sendPrompt).toHaveBeenCalledTimes(1);
      const [prompt] = sendPrompt.mock.calls[0] as unknown as [string];
      expect(prompt).toContain("1 source could not refresh");
      expect(prompt).toContain("Connection to the server failed: Cold storage");
      expect(prompt).not.toContain("10.0.0.5");
    } finally {
      useAssistantBridge.setState({ assistant: null });
    }
  });

  it("refuses to ask the assistant once another book is open", () => {
    const sendPrompt = vi.fn(() => true);
    useAssistantBridge.setState({
      assistant: {
        sendPrompt,
        captureBook: () => ({
          daemonSession: 1,
          scope: { workspace_id: "ws", profile_id: "pf-b" },
        }),
      } as never,
    });
    try {
      useNotificationStore.getState().raiseCondition(
        sourceFailure("s1", "Cold storage", "network I/O failed"),
      );
      openBell();
      const row = entry("1 source could not refresh");
      openMoreActions(row);
      // The list was rendered for book A; B opens before the click.
      currentBook = ACTIVE_B;
      act(() => {
        fireEvent.click(screen.getByRole("menuitem", { name: /Ask the assistant/ }));
      });
      expect(sendPrompt).not.toHaveBeenCalled();
    } finally {
      useAssistantBridge.setState({ assistant: null });
    }
  });

  it("offers no assistant when AI features are off", () => {
    useNotificationStore.getState().raiseCondition(
      sourceFailure("s1", "Cold storage", "Electrum timeout"),
    );
    openBell();
    const row = entry("1 source could not refresh");
    openMoreActions(row);
    expect(screen.queryByRole("menuitem", { name: /Ask the assistant/ })).toBeNull();
    expect(screen.getByRole("menuitem", { name: /Copy error details/ })).toBeTruthy();
  });

  it("shows the cause inline with one action, and extras behind More actions", () => {
    useNotificationStore.getState().raiseCondition(
      sourceFailure("s1", "Cold storage", "Electrum timeout"),
    );
    openBell();
    const row = entry("1 source could not refresh");
    // The cause in plain words, no toggle; the raw message is never shown.
    expect(
      within(row).getByText("Connection to the server failed: Cold storage"),
    ).toBeTruthy();
    expect(within(row).queryByText(/Electrum timeout/)).toBeNull();
    expect(within(row).queryByRole("button", { name: /Show details/ })).toBeNull();
    // One visible action; copy waits in the menu.
    expect(within(row).getByRole("button", { name: "Review sources" })).toBeTruthy();
    expect(within(row).queryByRole("button", { name: /Copy error details/ })).toBeNull();
    openMoreActions(row);
    expect(screen.getByRole("menuitem", { name: /Copy error details/ })).toBeTruthy();
  });

  it("dismisses one result and leaves the others", () => {
    const store = useNotificationStore.getState();
    store.addResult({ code: "report.exported", scope: { book: A }, severity: "info" });
    store.addResult({ code: "pricing.rebuilt", scope: { book: A }, severity: "info" });
    openBell();
    act(() => {
      fireEvent.click(
        screen.getByRole("button", { name: "Dismiss notification: Report exported" }),
      );
    });
    expect(useNotificationStore.getState().results.map((item) => item.code)).toEqual([
      "pricing.rebuilt",
    ]);
  });

  it("opens the exact connection, transaction, setting, and workspace", () => {
    const store = useNotificationStore.getState();
    store.raiseCondition({
      code: "connection.sync-failed",
      scope: { book: A, source: "connection:w-cold" },
      severity: "error",
      priority: "attention",
      action: { type: "open-connection", connectionId: "w-cold" },
    });
    store.raiseCondition({
      code: "source-funds.rescan-needed",
      scope: { book: A, source: "source-funds:tx-target" },
      severity: "warning",
      priority: "attention",
      params: { count: 1 },
      action: { type: "open-transaction", transactionId: "tx-old" },
    });
    store.raiseCondition({
      code: "touch-id.off",
      scope: { book: null },
      severity: "warning",
      priority: "attention",
      action: { type: "navigate", to: "/settings/security", anchor: "biometricUnlock" },
    });
    store.raiseCondition({
      code: "workspace.refresh-failed",
      scope: { book: null, source: "workspace:ws-1" },
      severity: "error",
      priority: "attention",
      action: { type: "open-workspace", workspaceId: "ws-1" },
    });
    openBell();
    act(() => {
      fireEvent.click(within(entry("A wallet could not sync")).getByRole("button", { name: "Open connection" }));
    });
    expect(navigate).toHaveBeenLastCalledWith({
      to: "/connections/$connectionId",
      params: { connectionId: "w-cold" },
    });
    reopen();
    act(() => {
      fireEvent.click(
        within(entry("A wallet needs a full rescan")).getByRole("button", { name: "Open transaction" }),
      );
    });
    expect(navigate).toHaveBeenLastCalledWith({
      to: "/transactions",
      search: { tx: "tx-old" },
    });
    reopen();
    act(() => {
      fireEvent.click(
        within(entry("Touch ID unlock is off")).getByRole("button", { name: "Enable Touch ID" }),
      );
    });
    expect(requestSettingsAnchor).toHaveBeenLastCalledWith("biometricUnlock");
    expect(navigate).toHaveBeenLastCalledWith({ to: "/settings/security" });
    reopen();
    act(() => {
      fireEvent.click(
        within(entry("Refreshing all books failed")).getByRole("button", { name: "Open all books" }),
      );
    });
    expect(navigate).toHaveBeenLastCalledWith({
      to: "/books/$workspaceId/birds-eye",
      params: { workspaceId: "ws-1" },
    });
  });

  it("speaks Austrian German for the moved senders", async () => {
    useNotificationStore.getState().raiseCondition({
      code: "connection.sync-failed",
      scope: { book: A, source: "connection:w-cold" },
      severity: "error",
      priority: "attention",
      action: { type: "open-connection", connectionId: "w-cold" },
    });
    await i18n.changeLanguage("de");
    useUiStore.setState({ lang: "de" });
    openBell();
    const row = entry("Sync einer Wallet fehlgeschlagen");
    expect(within(row).getByRole("button", { name: "Verbindung öffnen" })).toBeTruthy();
  });
});

describe("NotificationCenter privacy modes", () => {
  function seed() {
    const store = useNotificationStore.getState();
    store.raiseCondition(
      sourceFailure("s1", SENTINEL_LABEL, `rejected ${SENTINEL_AMOUNT} at ${SENTINEL_PATH}`),
    );
    store.raiseCondition(sourceFailure("s2", "Hot", "connection refused"));
    store.addResult({
      code: "report.exported",
      scope: { book: A },
      severity: "info",
      details: [{ label: SENTINEL_PATH, message: `${SENTINEL_AMOUNT} in 3 rows` }],
    });
  }

  function expandAll() {
    for (const toggle of screen.queryAllByRole("button", { name: /Show details/ })) {
      act(() => void fireEvent.click(toggle));
    }
  }

  it("Normal shows labels and bounded detail", () => {
    seed();
    openBell();
    expandAll();
    const text = renderedText();
    expect(text).toContain(SENTINEL_LABEL);
    expect(text).toContain("Connection to the server failed: Hot");
  });

  it("Hide amounts withholds free text that may hold an amount", () => {
    seed();
    useUiStore.setState({ privacyMode: "hideAmounts" });
    openBell();
    expandAll();
    const text = renderedText();
    expect(text).not.toContain(SENTINEL_AMOUNT);
    expect(text).not.toContain("0.4242");
    // Labels stay; the digit-bearing message does not.
    expect(text).toContain(`${SENTINEL_LABEL}: Hidden while amounts are hidden.`);
    expect(text).toContain("Connection to the server failed: Hot");
  });

  it("Hide amounts masks labels and progress text that carry an amount", () => {
    const store = useNotificationStore.getState();
    store.raiseCondition(sourceFailure("s3", "Savings (0.4242 BTC)", "timeout"));
    store.startJob({
      id: "book-refresh",
      code: "book-refresh",
      scope: { book: A },
      progress: { value: 40, label: "Syncing Savings (0.4242 BTC)" },
    });
    store.startJob({
      id: "connection-sync:w-savings",
      code: "connection-sync",
      scope: { book: A, source: "connection:w-savings" },
      progress: { value: 10, label: "Savings (0.4242 BTC): 3 of 9" },
    });

    openBell();
    expandAll();
    expect(renderedText()).toContain("Syncing Savings (0.4242 BTC)");
    cleanup();

    useUiStore.setState({ privacyMode: "hideAmounts" });
    openBell();
    expandAll();
    const text = renderedText();
    expect(text).not.toContain("0.4242");
    expect(text).toContain("Connection to the server failed: ••••");
    expect(within(entry("Refreshing book")).getByText("Hidden while amounts are hidden.")).toBeTruthy();
    expect(within(entry("Syncing wallet")).getByText("Hidden while amounts are hidden.")).toBeTruthy();
  });

  it("Hide all shows generic text, keeps actions, and leaks nothing", () => {
    seed();
    useUiStore.setState({ privacyMode: "hideAll" });
    const { runOperation } = openBell();
    expandAll();
    const text = renderedText();
    for (const sentinel of [SENTINEL_LABEL, SENTINEL_PATH, SENTINEL_AMOUNT, "Hot", "refused"]) {
      expect(text).not.toContain(sentinel);
    }
    expect(text).toContain("2 sources could not refresh");
    expect(text).toContain("Report exported");
    expect(screen.queryAllByRole("button", { name: /Show details/ })).toHaveLength(0);
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Review sources" }));
    });
    expect(navigate).toHaveBeenCalledWith({ to: "/connections" });
    expect(runOperation).not.toHaveBeenCalled();
  });
});

describe("notification persistence", () => {
  it("writes no notification payload to kb.ui", () => {
    useNotificationStore.getState().addResult({
      code: "report.exported",
      scope: { book: A },
      severity: "info",
      details: [{ label: SENTINEL_PATH, message: `${SENTINEL_AMOUNT} via tcp://10.0.0.7:50001` }],
      action: { type: "navigate", to: "/reports" },
    });
    useNotificationStore.getState().raiseCondition(refreshFailure(A));
    useNotificationStore.getState().raiseCondition(
      sourceFailure("s1", SENTINEL_LABEL, SENTINEL_PATH),
    );
    useUiStore.getState().setTheme("light");

    const stored = storage.get("kb.ui") ?? "";
    expect(stored).toContain('"theme":"light"');
    for (const sentinel of [
      SENTINEL_LABEL,
      SENTINEL_PATH,
      SENTINEL_AMOUNT,
      "10.0.0.7",
      "/settings/bitcoin",
      "notifications",
    ]) {
      expect(stored).not.toContain(sentinel);
    }
  });

  it("purges a legacy blob and keeps unrelated preferences", async () => {
    storage.set(
      "kb.ui",
      JSON.stringify({
        version: 4,
        state: {
          theme: "light",
          developerToolsEnabled: true,
          notifications: [
            {
              id: "old",
              createdAt: "2026-11-12T10:00:00Z",
              title: `Book refresh failed for ${SENTINEL_LABEL}`,
              body: SENTINEL_PATH,
              tone: "error",
              action: { label: "Retry", command: "refresh-book" },
            },
          ],
        },
      }),
    );
    // No store write between seeding and rehydrate: persist would overwrite it.
    await act(async () => {
      await useUiStore.persist.rehydrate();
    });

    const state = useUiStore.getState();
    expect((state as { notifications?: unknown }).notifications).toBeUndefined();
    expect(state.theme).toBe("light");
    expect(state.developerToolsEnabled).toBe(true);
    const stored = storage.get("kb.ui") ?? "";
    expect(JSON.parse(stored).version).toBe(UI_STATE_VERSION);
    expect(stored).not.toContain(SENTINEL_LABEL);
    expect(stored).not.toContain("refresh-book");
    expect(stored).not.toContain("notifications");

    // Nothing restored means nothing to run against the open book.
    const { runOperation } = openBell();
    expect(screen.queryByRole("button", { name: /Retry|Erneut/ })).toBeNull();
    expect(runOperation).not.toHaveBeenCalled();
  });
});
