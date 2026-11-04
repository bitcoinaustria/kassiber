// @vitest-environment happy-dom
//
// Transaction details keep their draft and selected tab when the panel hides
// or brings the assistant forward. Reopening never repeats a public lookup.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type * as React from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TabsContent } from "@/components/ui/tabs";

const mocks = vi.hoisted(() => ({ ask: vi.fn(), graphInvoke: vi.fn() }));

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

vi.mock("@/daemon/transport", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/daemon/transport")>()),
  getTransport: () => ({ invoke: mocks.graphInvoke }),
}));
vi.mock("@/daemon/client", async (importOriginal) => {
  const original = await importOriginal<typeof import("@/daemon/client")>();
  return {
    ...original,
    useDaemon: (kind: string, args?: Record<string, unknown>, options?: Parameters<typeof original.useDaemon>[2]) => kind === "ui.transactions.graph"
      ? original.useDaemon(kind, args, { ...options, retry: false, staleTime: 0 })
      : { data: undefined, isLoading: false, refetch: vi.fn() },
    useDaemonMutation: () => ({
      error: null,
      isPending: false,
      mutateAsync: vi.fn(),
      reset: vi.fn(),
    }),
  };
});
vi.mock("../TransactionClassifyTab", () => ({
  TransactionClassifyTab: ({ ctx }: { ctx: import("../TransactionDetailTabContext").TransactionDetailTabContext }) => (
    <TabsContent value="classify">
      <input aria-label="Transaction note" value={ctx.localDraft.note} onChange={(event) => ctx.updateDraft("note", event.target.value)} />
      <button onClick={ctx.enablePublicGraphLookup}>Lookup graph</button>
    </TabsContent>
  ),
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
import type { Transaction } from "@/components/transactions";
import { useUiStore } from "@/store/ui";
import { TransactionsTable } from "./TransactionsTable";

const TXN: Transaction = {
  id: "tx-1",
  txnId: "a".repeat(64),
  explorerId: "a".repeat(64),
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

// The menu hands focus back on a timer once it has unmounted.
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

describe("TransactionDetailSheet Ask about this", () => {
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
    mocks.graphInvoke.mockReset().mockResolvedValue({ kind: "ui.transactions.graph", data: undefined });
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

  it("keeps edits and the selected tab on reopen without repeating a failed public lookup", async () => {
    mocks.graphInvoke.mockImplementation(async ({ args }: { args: Record<string, unknown> }) => {
      if (args.allowPublicLookup) throw new Error("Lookup failed");
      return { kind: "ui.transactions.graph", data: undefined };
    });
    mount();
    fireEvent.click(screen.getAllByRole("row").find((row) => row.textContent?.includes("Exchange"))!);
    await settle();
    fireEvent.mouseDown(screen.getByRole("tab", { name: "Classify" }), { button: 0, ctrlKey: false });
    fireEvent.change(screen.getByRole("textbox", { name: "Transaction note" }), {
      target: { value: "Unsaved note" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Lookup graph" }));
    await waitFor(() => expect(mocks.graphInvoke).toHaveBeenCalledWith({
      kind: "ui.transactions.graph", args: { transaction: "tx-1", allowPublicLookup: true },
    }));
    await settle();
    act(() => useUiStore.getState().toggleRightPanel());
    await settle();
    mocks.graphInvoke.mockClear();
    act(() => useUiStore.getState().toggleRightPanel());
    await settle();
    expect(screen.getByRole("tab", { name: /^Classify/ }).getAttribute("aria-selected")).toBe("true");
    expect((screen.getByRole("textbox", { name: "Transaction note" }) as HTMLInputElement).value).toBe("Unsaved note");
    expect(mocks.graphInvoke).not.toHaveBeenCalledWith({
      kind: "ui.transactions.graph", args: { transaction: "tx-1", allowPublicLookup: true },
    });
    // Another explicit ask can retry the failed lookup.
    fireEvent.click(screen.getByRole("button", { name: "Lookup graph" }));
    await waitFor(() => expect(mocks.graphInvoke).toHaveBeenCalledWith({
      kind: "ui.transactions.graph", args: { transaction: "tx-1", allowPublicLookup: true },
    }));
  });

  it("keeps the detail as a tab and leaves focus in the assistant composer", async () => {
    mount();
    const row = screen
      .getAllByRole("row")
      .find((candidate) => candidate.textContent?.includes("Exchange"))!;
    act(() => {
      row.focus();
      fireEvent.click(row);
    });
    await settle();
    expect(document.querySelector('[data-right-panel][data-active-tab^="detail:"]')).not.toBeNull();

    const ask = screen.getByRole("button", { name: "Ask about this" });
    act(() => {
      ask.focus();
      fireEvent.click(ask);
    });
    // The sheet's focus scope hands focus back on a timer after unmounting.
    await settle();
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });

    expect(mocks.ask).toHaveBeenCalledWith({
      kind: "transaction",
      transactionId: "tx-1",
      date: TXN.date,
    });
    // One panel, two tabs: the assistant in front, the detail beside it,
    // named for its counterparty.
    const panel = document.querySelector<HTMLElement>("[data-right-panel]")!;
    expect(panel.getAttribute("data-active-tab")).toBe("assistant");
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((tab) => tab.textContent)).toEqual(["Assistant", "Exchange"]);
    expect(tabs[0].getAttribute("aria-selected")).toBe("true");
    expect(document.activeElement).toBe(screen.getByRole("textbox"));

    // The detail is still there, one click away.
    act(() => {
      fireEvent.click(tabs[1]);
    });
    expect(panel.getAttribute("data-active-tab")).toMatch(/^detail:/);
    expect(screen.getByRole("button", { name: "Ask about this" })).toBeTruthy();
  });
});
