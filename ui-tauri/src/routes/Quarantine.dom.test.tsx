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
  sheetFingerprint: "fp-1",
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
                workspace_id: "ws",
                profile_id: "book",
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
                // The sheet's pair is a journal relation; it names the review
                // behind it and that review's fingerprint as read now.
                ...(id === "out" || id === "in"
                  ? {
                      pair: {
                        id: "rel-1",
                        type: "transfer",
                        kind: "manual",
                        reviewPairId: "pair-1",
                        pairFingerprint: daemon.sheetFingerprint,
                      },
                    }
                  : {}),
              },
              workspaceId: "ws",
              profileId: "book",
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
      onUnpair?: (pairId: string) => void;
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
          {props.onUnpair ? (
            <button type="button" onClick={() => props.onUnpair?.("rel-1")}>
              Unpair from the sheet
            </button>
          ) : null}
        </div>
      ) : null,
  };
});

import { useUiStore } from "@/store/ui";

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
  daemon.sheetFingerprint = "fp-1";
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

  it("unpairs from the sheet only through the reviewed, book-bound step", async () => {
    const legs = {
      out: { transaction_id: "out", wallet: "Merchant", asset: "BTC", amount_msat: 100_000_000, occurred_at: "2024-01-01T00:00:00Z", external_id: "a".repeat(64) },
      in: { transaction_id: "in", wallet: "Spending", asset: "BTC", amount_msat: 99_000_000, occurred_at: "2024-01-02T00:00:00Z", external_id: "b".repeat(64) },
    };
    const pairRoot = row("out", {
      reason: "custody_quantity_unresolved",
      category: "needs_decision",
      evidence: {
        blocker_code: "reviewed_residual_suspense",
        pair_id: "pair-1",
        pair_fingerprint: "fp-1",
        pair_counterpart_transaction_id: "in",
        pair_legs: legs,
      },
      actions: [{ kind: "review_pair", transaction_id: "out", pair_id: "pair-1" }],
    });
    const plain = row("plain");
    daemon.groups = [cause("pairs", [pairRoot]), cause("price", [plain])];
    daemon.items = [pairRoot, plain];
    mount();

    // A row that is not a pair case is not offered an Unpair at all.
    await showScope(/All/);
    fireEvent.click((await screen.findByText(/Wallet plain/)).closest("button")!);
    await screen.findByTestId("sheet");
    expect(screen.queryByText("Unpair from the sheet")).toBeNull();

    fireEvent.click(screen.getAllByText("Sent")[0].closest("button")!);
    await waitFor(() =>
      expect(screen.getByTestId("sheet").getAttribute("data-transaction")).toBe("out"),
    );
    await act(async () => {
      fireEvent.click(screen.getByText("Unpair from the sheet"));
    });
    await waitFor(() =>
      expect(daemon.calls.some((call) => call.kind === "ui.review.plan")).toBe(true),
    );
    const plan = daemon.calls.find((call) => call.kind === "ui.review.plan")!;
    expect(plan.args.expected_scope).toEqual({ workspace_id: "ws", profile_id: "book" });
    expect(plan.args.operations).toEqual([
      expect.objectContaining({ type: "unpair", pair_id: "pair-1", expected_fingerprint: "fp-1" }),
    ]);
    expect(daemon.calls.some((call) => call.kind === "ui.transfers.unpair")).toBe(false);
  });

  it("refuses a sheet unpair when the page reads the pair newer than the sheet shows it", async () => {
    const legs = {
      out: { transaction_id: "out", wallet: "Merchant", asset: "BTC", amount_msat: 100_000_000, occurred_at: "2024-01-01T00:00:00Z", external_id: "a".repeat(64) },
      in: { transaction_id: "in", wallet: "Spending", asset: "BTC", amount_msat: 99_000_000, occurred_at: "2024-01-02T00:00:00Z", external_id: "b".repeat(64) },
    };
    // The sheet shows P as reviewed `manual` (fp-1); the page's attention
    // read already holds P revised to `coinjoin` (fp-2) by another session.
    const pairRoot = row("out", {
      reason: "custody_quantity_unresolved",
      category: "needs_decision",
      evidence: {
        blocker_code: "reviewed_residual_suspense",
        pair_id: "pair-1",
        pair_fingerprint: "fp-2",
        pair_counterpart_transaction_id: "in",
        pair_legs: legs,
        pair_review: { kind: "coinjoin", policy: "carrying-value", out_amount_msat: 99_000_000, in_amount_msat: 99_000_000 },
      },
      actions: [{ kind: "review_pair", transaction_id: "out", pair_id: "pair-1" }],
    });
    daemon.groups = [cause("pairs", [pairRoot])];
    daemon.items = [pairRoot];
    daemon.sheetFingerprint = "fp-1";
    mount();
    fireEvent.click((await screen.findAllByText("Sent"))[0].closest("button")!);
    await screen.findByText("Unpair from the sheet");
    await act(async () => {
      fireEvent.click(screen.getByText("Unpair from the sheet"));
    });

    expect(daemon.calls.some((call) => call.kind === "ui.review.plan")).toBe(false);
    expect(daemon.calls.some((call) => call.kind === "ui.transfers.unpair")).toBe(false);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(
      useUiStore.getState().notifications.some((entry) =>
        String(entry.body).includes("reads differently now than the sheet showed it"),
      ),
    ).toBe(true);
  });
});
