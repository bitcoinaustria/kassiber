// @vitest-environment happy-dom
//
// Mounted with the real query layer: list failures and "Save & next" both
// depend on what a refetch returns, which a static render never runs.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { QuarantineItem } from "@/components/kb/quarantine";

const daemon = vi.hoisted(() => ({
  items: [] as unknown[],
  groups: [] as unknown[],
  failWaiting: false,
  calls: [] as Array<{ kind: string; args: Record<string, unknown> }>,
}));

vi.mock("@/daemon/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/transport")>();
  return {
    ...actual,
    getTransport: () => ({
      invoke: async ({ kind, args = {} }: { kind: string; args?: Record<string, unknown> }) => {
        daemon.calls.push({ kind, args });
        if (kind === "ui.journals.quarantine") {
          const scope = (args.scope as string) ?? "all";
          if (scope === "waiting" && daemon.failWaiting) {
            return {
              kind: "error",
              schema_version: 1,
              error: { code: "internal", message: "The waiting list broke", retryable: false },
            };
          }
          const items = (daemon.items as QuarantineItem[]).filter((item) =>
            scope === "all" ? true : scope === "waiting" ? Boolean(item.root) : !item.root,
          );
          return {
            kind,
            schema_version: 1,
            data: {
              summary: {
                count: daemon.items.length,
                attention_count: daemon.items.length - (daemon.items as QuarantineItem[]).filter((item) => item.root).length,
                waiting_count: (daemon.items as QuarantineItem[]).filter((item) => item.root).length,
                scope,
                scope_count: items.length,
                offset: Number(args.offset ?? 0),
                limit: Number(args.limit ?? 100),
                groups: daemon.groups,
              },
              // Pages like the daemon: at most `limit` rows from `offset`.
              items: items.slice(Number(args.offset ?? 0), Number(args.offset ?? 0) + Number(args.limit ?? 100)),
            },
          };
        }
        if (kind === "ui.transactions.resolve") {
          const id = String(args.query);
          return {
            kind,
            schema_version: 1,
            data: {
              transaction: {
                id,
                date: "2024-01-01 00:00",
                type: "Transfer",
                account: `Wallet ${id}`,
                counter: "Transfer",
                amountSat: 1000,
                eur: null,
                rate: null,
                tag: "Transfer",
                conf: 3,
                feeSat: 0,
              },
            },
          };
        }
        return { kind, schema_version: 1, data: {} };
      },
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));

vi.mock("@/hooks/useJournalProcessingAction", () => ({
  useJournalProcessingAction: () => ({ runJournalProcessing: vi.fn(), isProcessingJournals: false }),
}));

// The sheet is a large surface of its own; this stub shows which row it
// opened, on which tab and with which reason, and offers "Save & next".
vi.mock("@/components/transactions", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/components/transactions")>();
  return {
    ...actual,
    TransactionDetailSheet: (props: {
      transaction: { id: string } | null;
      draft: unknown;
      initialTab: string;
      quarantineReasonOverride: string | null;
      hasNext?: boolean;
      onSaveAndNext?: (id: string, draft: unknown) => Promise<void>;
    }) =>
      props.transaction ? (
        <div
          data-testid="sheet"
          data-transaction={props.transaction.id}
          data-tab={props.initialTab}
          data-reason={props.quarantineReasonOverride ?? ""}
        >
          {props.hasNext ? (
            <button
              type="button"
              onClick={() => void props.onSaveAndNext?.(props.transaction!.id, props.draft)}
            >
              Save and open next
            </button>
          ) : null}
        </div>
      ) : null,
  };
});

import { Quarantine } from "./Quarantine";

function row(transactionId: string, overrides: Partial<QuarantineItem> = {}): QuarantineItem {
  return {
    transaction_id: transactionId,
    external_id: "",
    occurred_at: "2024-01-01T00:00:00Z",
    confirmed_at: null,
    wallet: `Wallet ${transactionId}`,
    direction: "outbound",
    asset: "BTC",
    amount: 0.00001,
    amount_msat: 1_000_000,
    fee: 0,
    fee_msat: 0,
    reason: "missing_spot_price",
    detail: {},
    created_at: "2026-01-01T00:00:00Z",
    category: "missing_price",
    blocks_reports: false,
    is_downstream: false,
    root: null,
    evidence: {},
    actions: [],
    ...overrides,
  };
}

function waitingRow(transactionId: string): QuarantineItem {
  return row(transactionId, {
    reason: "custody_basis_barrier",
    category: "downstream",
    is_downstream: true,
    root: {
      transaction_id: "root",
      reason: "missing_spot_price",
      occurred_at: "2024-01-01T00:00:00Z",
      wallet: "Wallet root",
      external_id: "",
    },
  });
}

function mount() {
  const client = new QueryClient({
    defaultOptions: { queries: { retryDelay: 0 }, mutations: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <Quarantine />
    </QueryClientProvider>,
  );
}

/** Opens the list of rows that only wait on a cause. */
async function showWaiting() {
  fireEvent.click(await screen.findByRole("button", { name: "Show them" }));
}

/** One cause holding the given root rows, as the daemon groups them. */
function cause(key: string, roots: QuarantineItem[]) {
  for (const item of roots) item.group_key = key;
  const first = roots[0];
  return {
    key,
    category: first.category,
    reason: first.reason,
    root_transaction_id: first.transaction_id,
    count: roots.length,
    downstream_count: 0,
    blocks_reports: false,
    evidence: first.evidence,
    actions: [],
    root_transaction_ids: roots.slice(0, 25).map((item) => item.transaction_id),
    root_count: roots.length,
  };
}

beforeEach(() => {
  daemon.items = [];
  daemon.groups = [];
  daemon.failWaiting = false;
  daemon.calls = [];
  window.history.replaceState(null, "", "/quarantine");
});

afterEach(cleanup);

describe("quarantine route", () => {
  it("shows a failed waiting list as an error with retry, not an empty queue", async () => {
    daemon.items = [row("root"), waitingRow("w1")];
    daemon.failWaiting = true;
    mount();
    await showWaiting();

    const alert = await screen.findByRole("alert", {}, { timeout: 3000 });
    expect(alert.textContent).toContain("This list could not be loaded.");
    expect(alert.textContent).toContain("The waiting list broke");
    expect(screen.queryByText("No transaction is waiting on another.")).toBeNull();

    daemon.failWaiting = false;
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    });
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
    expect(screen.getByText(/Wallet w1/)).toBeTruthy();
  });

  it("opens the next row from the refreshed list when the old next one cleared", async () => {
    daemon.items = [row("root"), waitingRow("w-a"), waitingRow("w-b"), waitingRow("w-c")];
    mount();
    await showWaiting();
    fireEvent.click((await screen.findByText(/Wallet w-a/)).closest("button")!);
    await screen.findByText("Save and open next");

    // A sync cleared "w-b" while "w-a" was being edited.
    daemon.items = [row("root"), waitingRow("w-a"), waitingRow("w-c")];
    await act(async () => {
      fireEvent.click(screen.getByText("Save and open next"));
    });

    await waitFor(() =>
      expect(screen.getByTestId("sheet").getAttribute("data-transaction")).toBe("w-c"),
    );
    expect(
      daemon.calls.some(
        (call) => call.kind === "ui.transactions.resolve" && call.args.query === "w-b",
      ),
    ).toBe(false);
  });

  it("opens the next row with its refreshed reason, not the one it was listed with", async () => {
    const roots = [row("a"), row("b"), row("c")];
    daemon.groups = [cause("price", roots)];
    daemon.items = roots;
    mount();
    fireEvent.click((await screen.findByText(/Wallet a/)).closest("button")!);
    await screen.findByText("Save and open next");

    // "b" now needs its acquisition history rather than a price.
    daemon.items = [
      roots[0],
      { ...roots[1], reason: "insufficient_lots", category: "missing_acquisition_history" },
      roots[2],
    ];
    await act(async () => {
      fireEvent.click(screen.getByText("Save and open next"));
    });

    await waitFor(() =>
      expect(screen.getByTestId("sheet").getAttribute("data-transaction")).toBe("b"),
    );
    const sheet = screen.getByTestId("sheet");
    expect(sheet.getAttribute("data-reason")).toBe("insufficient_lots");
    expect(sheet.getAttribute("data-tab")).toBe("tax");
  });

  it("loads more of a cause past the first page without resolving that page first", async () => {
    const roots = Array.from({ length: 150 }, (_, index) => row(`r-${String(index + 1).padStart(3, "0")}`));
    daemon.groups = [cause("price", roots)];
    daemon.items = roots;
    mount();
    expect(await screen.findByText("50 more transactions of this cause are not loaded yet.")).toBeTruthy();
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Load more" }));
    });
    await waitFor(() => expect(screen.queryByText(/not loaded yet/)).toBeNull());
    fireEvent.click(screen.getByRole("button", { name: "Show all 150 transactions" }));
    // Row 120 sits on the second page and opens like any other.
    fireEvent.click(screen.getByText(/Wallet r-120/).closest("button")!);
    await waitFor(() =>
      expect(screen.getByTestId("sheet").getAttribute("data-transaction")).toBe("r-120"),
    );
    const offsets = daemon.calls
      .filter((call) => call.kind === "ui.journals.quarantine" && call.args.scope === "attention")
      .map((call) => call.args.offset);
    expect(offsets).toEqual([0, 100]);
  });

  it("walks the opened cause's loaded rows, past the 25 the summary names", async () => {
    const roots = Array.from({ length: 30 }, (_, index) => row(`r-${String(index + 1).padStart(3, "0")}`));
    daemon.groups = [cause("price", roots)];
    daemon.items = roots;
    mount();
    fireEvent.click(await screen.findByRole("button", { name: "Show all 30 transactions" }));
    fireEvent.click(screen.getByText(/Wallet r-026/).closest("button")!);
    await screen.findByText("Save and open next");
    await act(async () => {
      fireEvent.click(screen.getByText("Save and open next"));
    });
    await waitFor(() =>
      expect(screen.getByTestId("sheet").getAttribute("data-transaction")).toBe("r-027"),
    );
  });
});
