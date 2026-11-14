// @vitest-environment happy-dom
//
// A reviewed own-wallet move arrives from `ui.transactions.list` as one row
// whose amount is the pair's fee (zero for a plain move) and whose counter
// reads "Transfer fee - BTC -> BTC". The table names the move and shows what
// moved, with a recorded pair fee on its own line.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, within } from "@testing-library/react";
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
    attachments: [],
    attachFile: vi.fn(),
    removeAttachment: vi.fn(),
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
import type { Tx } from "@/mocks/seed";
import { toDashboardTransaction } from "./model";
import { TransactionsTable } from "./TransactionsTable";

function barkSweep(pair: Partial<NonNullable<Tx["pair"]>> = {}): Tx {
  return {
    id: "170cb33f-43c4-49f5-9885-562ecaf3a079",
    externalId: "6bb93c0819b689af5cc948d718c3978671d832ff764c6ab73702e5d97edce2ea",
    explorerId: "6bb93c0819b689af5cc948d718c3978671d832ff764c6ab73702e5d97edce2ea",
    date: "2026-11-13 11:52",
    type: "Transfer",
    kind: "deposit",
    asset: "BTC",
    chain: "bitcoin",
    network: "regtest",
    paymentMethod: "On-chain",
    account: "BTCPay Bark -> Store on-chain",
    counter: "Transfer fee - BTC -> BTC",
    amountSat: 0,
    feeSat: pair.feeSat ?? 0,
    eur: -0,
    rate: 92_000,
    fiatCurrency: "EUR",
    queueStatus: "completed",
    tag: "Transfer",
    conf: 1,
    internal: true,
    pair: {
      id: "c7a09aab",
      type: "transfer",
      kind: "bark_txid_exact",
      policy: "carrying-value",
      outWallet: "BTCPay Bark",
      outAsset: "BTC",
      outAmountSat: 600_000,
      inWallet: "Store on-chain",
      inAsset: "BTC",
      inAmountSat: 600_000,
      feeSat: 0,
      feeKind: null,
      ...pair,
    },
  };
}

function mount(tx: Tx, currency: "btc" | "eur") {
  const queryClient = new QueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <AssistantLayout enabled={false}>
        <main>
          <TransactionsTable
            records={[toDashboardTransaction(tx, 0)]}
            hideSensitive={false}
            currency={currency}
            nowRate={92_000}
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

function sweepRow() {
  const row = screen
    .queryAllByRole("row")
    .find((candidate) => candidate.textContent?.includes("Store on-chain"));
  if (!row) throw new Error("paired transfer row missing");
  return row;
}

describe("TransactionsTable paired transfers", () => {
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

  it("names the move and shows the moved amount, not the zero pair fee", () => {
    mount(barkSweep(), "btc");
    const row = sweepRow();
    expect(
      within(row).getByText("Transfer · BTCPay Bark → Store on-chain"),
    ).toBeTruthy();
    expect(row.textContent).not.toContain("Transfer fee");
    expect(within(row).getByText("₿0.00600000")).toBeTruthy();
    expect(within(row).getByText("€ 552,00")).toBeTruthy();
    expect(row.textContent).not.toContain("₿0.00000000");
    expect(row.textContent).not.toMatch(/Fee /);
  });

  it("puts a recorded pair fee on its own line in the display currency", () => {
    mount(
      barkSweep({ outAmountSat: 600_420, inAmountSat: 600_000, feeSat: 420 }),
      "btc",
    );
    expect(within(sweepRow()).getByText("Fee ₿0.00000420")).toBeTruthy();
    cleanup();

    mount(
      barkSweep({ outAmountSat: 600_420, inAmountSat: 600_000, feeSat: 420 }),
      "eur",
    );
    const row = sweepRow();
    expect(within(row).getByText("€ 552,00")).toBeTruthy();
    expect(within(row).getByText("₿ 0.00600000")).toBeTruthy();
    expect(within(row).getByText("Fee € 0,39")).toBeTruthy();
  });
});
