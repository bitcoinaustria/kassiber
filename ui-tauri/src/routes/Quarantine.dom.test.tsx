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
                groups: daemon.groups,
              },
              items,
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

/** Unfolds the list and picks a scope; Radix tabs switch on mouse down. */
async function showScope(name: RegExp) {
  fireEvent.click(await screen.findByRole("button", { name: /^Show/ }));
  fireEvent.mouseDown(await screen.findByRole("tab", { name }), { button: 0 });
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
    await showScope(/Waiting/);

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
    daemon.items = [row("a"), row("b"), row("c")];
    mount();
    await showScope(/All/);
    fireEvent.click((await screen.findByText(/Wallet a/)).closest("button")!);
    await screen.findByText("Save and open next");

    // A sync cleared "b" while "a" was being edited.
    daemon.items = [row("a"), row("c")];
    await act(async () => {
      fireEvent.click(screen.getByText("Save and open next"));
    });

    await waitFor(() =>
      expect(screen.getByTestId("sheet").getAttribute("data-transaction")).toBe("c"),
    );
    expect(
      daemon.calls.some(
        (call) => call.kind === "ui.transactions.resolve" && call.args.query === "b",
      ),
    ).toBe(false);
  });

  it("opens the next row with its refreshed reason, not the one it was listed with", async () => {
    daemon.items = [row("a"), row("b"), row("c")];
    mount();
    await showScope(/All/);
    fireEvent.click((await screen.findByText(/Wallet a/)).closest("button")!);
    await screen.findByText("Save and open next");

    // "b" now needs its acquisition history rather than a price.
    daemon.items = [
      row("a"),
      row("b", { reason: "insufficient_lots", category: "missing_acquisition_history" }),
      row("c"),
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

  it("unpairs a pair only as the owner confirmed it, after a fresh read", async () => {
    const legs = {
      out: { transaction_id: "out", wallet: "Merchant", asset: "BTC", amount_msat: 100_000_000, occurred_at: "2024-01-01T00:00:00Z", external_id: "a".repeat(64) },
      in: { transaction_id: "in", wallet: "Spending", asset: "BTC", amount_msat: 99_000_000, occurred_at: "2024-01-02T00:00:00Z", external_id: "b".repeat(64) },
    };
    const review = { kind: "manual", policy: "carrying-value", out_amount_msat: 99_000_000, in_amount_msat: 99_000_000 };
    const evidence = {
      blocker_code: "reviewed_residual_suspense",
      pair_id: "pair-1",
      pair_txids_differ: true,
      pair_legs: legs,
      pair_review: review,
    };
    const key = "custody_quantity_unresolved:reviewed_residual_suspense:";
    const pairRoot = row("out", {
      reason: "custody_quantity_unresolved",
      category: "needs_decision",
      blocks_reports: true,
      evidence,
      actions: [{ kind: "review_pair", transaction_id: "out", pair_id: "pair-1" }],
      group_key: key,
    });
    daemon.items = [pairRoot];
    daemon.groups = [
      {
        key,
        category: "needs_decision",
        reason: "custody_quantity_unresolved",
        root_transaction_id: "out",
        count: 1,
        downstream_count: 0,
        blocks_reports: true,
        evidence,
        actions: pairRoot.actions,
        root_transaction_ids: ["out"],
        root_count: 1,
      },
    ];
    mount();
    const unpairCalls = () => daemon.calls.filter((call) => call.kind === "ui.transfers.unpair");

    // Another session revised the pair after the card was drawn.
    fireEvent.click(await screen.findByRole("button", { name: "Unpair" }));
    daemon.items = [{ ...pairRoot, evidence: { ...evidence, pair_review: { ...review, kind: "swap_refund" } } }];
    await act(async () => {
      fireEvent.click(await screen.findByRole("button", { name: "Unpair and recalculate" }));
    });
    expect((await screen.findByRole("alert")).textContent).toContain("This pair changed since it was listed");
    expect(unpairCalls()).toEqual([]);

    // Looked at again, the card shows the pair as it now reads; confirmed
    // as such, exactly that pair is removed.
    fireEvent.click(screen.getAllByRole("button", { name: "Close" })[0]);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    fireEvent.click(await screen.findByRole("button", { name: "Unpair" }));
    await act(async () => {
      fireEvent.click(await screen.findByRole("button", { name: "Unpair and recalculate" }));
    });
    await waitFor(() => expect(unpairCalls()).toHaveLength(1));
    expect(unpairCalls()[0].args).toEqual({ pair_id: "pair-1" });
  });
});
