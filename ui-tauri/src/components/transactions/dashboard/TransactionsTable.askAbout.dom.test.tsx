// @vitest-environment happy-dom
//
// "Ask about this" from a transaction row's menu: the panel opens with focus
// in its composer (the closing menu must not hand focus back to its trigger),
// and the click must not reach the row, which would open the detail sheet.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import type * as React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ ask: vi.fn() }));

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
  globalThis.ResizeObserver ??= class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
});

vi.mock("@/daemon/client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/daemon/client")>()),
  useDaemon: () => ({ data: undefined, isLoading: false, refetch: vi.fn() }),
  useDaemonMutation: () => ({
    error: null,
    isPending: false,
    mutateAsync: vi.fn(),
    reset: vi.fn(),
  }),
}));
vi.mock("@/hooks/useJournalProcessingAction", () => ({
  useJournalProcessingAction: () => ({
    runJournalProcessing: vi.fn(),
    isProcessingJournals: false,
  }),
}));
vi.mock("./useTransactionDetailRecord", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./useTransactionDetailRecord")>()),
  useTransactionDetailRecord: (record: unknown) => ({
    record,
    isLoading: false,
    retry: vi.fn(),
  }),
}));
// Every row rendered: happy-dom has no layout for the virtualizer to measure.
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        key: index,
        start: index * 76,
        end: (index + 1) * 76,
        size: 76,
        lane: 0,
      })),
    getTotalSize: () => count * 76,
    measureElement: () => {},
  }),
}));
vi.mock("@/components/transactions", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/transactions")>()),
  TransactionDetailSheet: ({ open }: { open: boolean }) =>
    open ? <div data-testid="detail-sheet" /> : null,
}));
vi.mock("@/components/ai/useAskAssistantAbout", () => ({
  useAskAssistantAbout: () => mocks.ask,
}));
vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  Link: ({ children, ...props }: React.ComponentProps<"a">) => (
    <a {...props}>{children}</a>
  ),
}));
vi.mock("@/components/ai/assistantSession", () => ({
  useAssistantSession: () => ({
    messages: [],
    isStreaming: false,
    abort: vi.fn(),
    error: null,
    pendingConsents: [],
    queuedPrompts: [],
    sendConsent: vi.fn(),
    selection: { provider: "copilot", model: "default" },
    setSelection: vi.fn(),
    thinkingEffort: "auto",
    setThinkingEffort: vi.fn(),
    fastMode: false,
    setFastMode: vi.fn(),
    sendPrompt: vi.fn(),
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

import { AssistantLayout } from "@/components/kb/assistant-panel/AssistantLayout";
import type { Transaction } from "@/components/transactions";
import { useUiStore } from "@/store/ui";
import { TransactionsTable } from "./TransactionsTable";

const TXN: Transaction = {
  id: "tx-1",
  txnId: "a".repeat(64),
  amount: 50,
  amountBtc: 0.001,
  counterparty: "Exchange",
  counterpartyInitials: "EX",
  direction: "Receive",
  flow: "incoming",
  paymentMethod: "On-chain",
  date: "2026-03-01 10:00",
  status: "completed",
};

function mount() {
  const queryClient = new QueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <AssistantLayout enabled>
        <main>
          <TransactionsTable
            records={[TXN]}
            hideSensitive={false}
            currency="btc"
            nowRate={60_000}
            explorerSettings={{} as never}
            chartSelection={null}
            quickFilter={null}
            breakdownSelection={null}
            onTransactionIdFilterChange={() => {}}
            onChartSelectionChange={() => {}}
            onQuickFilterChange={() => {}}
            onBreakdownSelectionChange={() => {}}
            filterState={{
              status: "all",
              flow: "all",
              paymentMethod: "all",
              fee: "all",
              sort: null,
            }}
            onFilterStateChange={() => {}}
          />
        </main>
      </AssistantLayout>
    </QueryClientProvider>,
  );
}

function openRowMenu() {
  const trigger = screen.getByRole("button", {
    name: `Open actions for ${TXN.txnId}`,
  });
  act(() => {
    trigger.focus();
    fireEvent.keyDown(trigger, { key: "Enter" });
  });
  return trigger;
}

// The menu hands focus back on a timer once it has unmounted.
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

describe("TransactionsTable row menu", () => {
  beforeEach(() => {
    window.history.replaceState(null, "", "/transactions");
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: (query: string) => ({
        matches: false,
        media: query,
        addEventListener: () => {},
        removeEventListener: () => {},
      }),
    });
    mocks.ask.mockReset();
    // What the real session does: open the panel and ask for the composer.
    mocks.ask.mockImplementation(() =>
      useUiStore.getState().openAssistantPanel({ focusComposer: true }),
    );
    useUiStore.setState({
      assistantPanelOpen: false,
      rightPanelOpen: false,
      assistantClosedBar: "launcher",
    });
  });
  afterEach(cleanup);

  it("asks about the row and leaves focus in the panel composer", async () => {
    mount();
    openRowMenu();
    act(() => {
      fireEvent.click(screen.getByRole("menuitem", { name: "Ask about this" }));
    });
    await settle();

    expect(mocks.ask).toHaveBeenCalledWith({
      kind: "transaction",
      transactionId: "tx-1",
      date: TXN.date,
    });
    expect(
      document.querySelector('[data-right-panel][data-active-tab="assistant"]'),
    ).not.toBeNull();
    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(screen.getByRole("textbox"));
    // The click stayed in the menu: the row behind it did not open its sheet.
    expect(screen.queryByTestId("detail-sheet")).toBeNull();
    expect(window.location.search).not.toContain("tx=");
  });

  it("returns focus to the trigger when the ask is refused", async () => {
    mocks.ask.mockImplementation(() => {});
    mount();
    const trigger = openRowMenu();
    act(() => {
      fireEvent.click(screen.getByRole("menuitem", { name: "Ask about this" }));
    });
    await settle();

    expect(document.querySelector("[data-right-panel]")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("keeps other row actions from opening the detail sheet", async () => {
    mount();
    openRowMenu();
    act(() => {
      fireEvent.click(screen.getByRole("menuitem", { name: /Copy/ }));
    });
    await settle();
    expect(screen.queryByTestId("detail-sheet")).toBeNull();
  });
});
