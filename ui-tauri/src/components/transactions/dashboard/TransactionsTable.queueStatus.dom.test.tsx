// @vitest-environment happy-dom
//
// The table's status-based quick filters (Review queue, Failed import) and
// its status filter follow the status the review queue counts a row under,
// not the status the user authored, so opening the Review queue card shows
// the rows it counts.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen } from "@testing-library/react";
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
import { TransactionsTable } from "./TransactionsTable";

function record(overrides: Partial<Transaction>): Transaction {
  return {
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
    ...overrides,
  };
}

function mount(
  records: Transaction[],
  quickFilter: "review_queue" | "failed_import" | null,
) {
  const queryClient = new QueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <AssistantLayout enabled={false}>
        <main>
          <TransactionsTable
            records={records}
            hideSensitive={false}
            currency="btc"
            nowRate={60_000}
            explorerSettings={{} as never}
            chartSelection={null}
            quickFilter={quickFilter}
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

function rowFor(counterparty: string) {
  return screen
    .queryAllByRole("row")
    .find((row) => row.textContent?.includes(counterparty));
}

describe("TransactionsTable queue status filters", () => {
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
  });
  afterEach(cleanup);

  it("shows a held row the user marked completed in the Review queue", () => {
    mount(
      [
        record({
          id: "held",
          counterparty: "Held in quarantine",
          reviewStatus: "completed",
          queueStatus: "review",
          quarantineReason: "missing_price",
        }),
        record({
          id: "done",
          txnId: "b".repeat(64),
          counterparty: "Nothing to do",
          reviewStatus: "completed",
          queueStatus: "completed",
        }),
      ],
      "review_queue",
    );
    expect(rowFor("Held in quarantine")).toBeTruthy();
    expect(rowFor("Nothing to do")).toBeUndefined();
  });

  it("finds a failed row by the status the queue counts", () => {
    mount(
      [
        record({
          id: "failed-pair",
          counterparty: "Failed pair",
          reviewStatus: "completed",
          queueStatus: "failed",
        }),
      ],
      "failed_import",
    );
    expect(rowFor("Failed pair")).toBeTruthy();
  });
});
