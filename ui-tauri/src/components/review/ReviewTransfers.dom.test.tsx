// @vitest-environment happy-dom
//
// Mounted with the real query layer: hiding a decided candidate, undoing it
// and reading the list again after a recalculation all run through effects.
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import "@/i18n";

import type { TransferCandidate } from "./transferModel";

const daemon = vi.hoisted(() => ({
  candidates: [] as unknown[],
  failUnpair: new Set<string>(),
  failPair: false,
  calls: [] as Array<{ kind: string; args: Record<string, unknown> }>,
}));

vi.mock("@/daemon/transport", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/daemon/transport")>();
  return {
    ...actual,
    getTransport: () => ({
      invoke: async ({ kind, args = {} }: { kind: string; args?: Record<string, unknown> }) => {
        daemon.calls.push({ kind, args });
        if (kind === "ui.transfers.suggest") {
          const candidates = daemon.candidates as TransferCandidate[];
          return {
            kind,
            schema_version: 1,
            data: {
              candidates,
              counts: { total: candidates.length, exact: 0, strong: 0, conflicts: 0 },
            },
          };
        }
        if (kind === "ui.transfers.pair" && daemon.failPair) {
          return {
            kind: "error",
            schema_version: 1,
            error: { code: "conflict", message: "That leg is paired already", retryable: false },
          };
        }
        if (kind === "ui.transfers.pair") return { kind, schema_version: 1, data: { id: "pair-1" } };
        if (kind === "ui.transfers.unpair" && daemon.failUnpair.has(String(args.pair_id))) {
          return {
            kind: "error",
            schema_version: 1,
            error: { code: "conflict", message: "That pair changed", retryable: false },
          };
        }
        if (kind === "ui.transfers.dismiss") return { kind, schema_version: 1, data: { id: "dismissal-1" } };
        if (kind === "ui.transfers.bulk_pair") {
          return {
            kind,
            schema_version: 1,
            data: {
              applied: [
                { id: "pair-a", out_transaction_id: "out-a", in_transaction_id: "in-a" },
                { id: "pair-b", out_transaction_id: "out-b", in_transaction_id: "in-b" },
              ],
              summary: { count: 2, skipped_conflicts: 0, total_swap_fee_msat: 0 },
            },
          };
        }
        return { kind, schema_version: 1, data: {} };
      },
    }),
  };
});

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { children: React.ReactNode; to: string }) => <a href={to}>{children}</a>,
}));

import { ReviewTransfers } from "./ReviewTransfers";

function candidate(id: string, overrides: Partial<TransferCandidate> = {}): TransferCandidate {
  return {
    out_id: `out-${id}`,
    in_id: `in-${id}`,
    out_asset: "BTC",
    in_asset: "BTC",
    out_amount_msat: 5_000_000_000,
    in_amount_msat: 4_990_000_000,
    out_wallet_label: `Hot ${id}`,
    in_wallet_label: `Cold ${id}`,
    out_occurred_at: "2024-05-24T09:51:00Z",
    in_occurred_at: "2024-05-24T10:01:00Z",
    confidence: "strong",
    method: "heuristic",
    swap_fee_msat: 10_000_000,
    default_kind: "manual",
    default_policy: "carrying-value",
    candidate_type: "transfer",
    conflict_set_id: id,
    conflict_size: 1,
    ...overrides,
  };
}

function mount(props: { refreshing?: boolean; onDecided?: () => void } = {}) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const onDecided = props.onDecided ?? vi.fn();
  const view = render(
    <QueryClientProvider client={client}>
      <ReviewTransfers onDecided={onDecided} refreshing={props.refreshing ?? false} />
    </QueryClientProvider>,
  );
  return {
    onDecided,
    rerender: (refreshing: boolean) =>
      view.rerender(
        <QueryClientProvider client={client}>
          <ReviewTransfers onDecided={onDecided} refreshing={refreshing} />
        </QueryClientProvider>,
      ),
  };
}

const rows = () => screen.queryAllByRole("listitem");

beforeEach(() => {
  daemon.candidates = [candidate("a"), candidate("b")];
  daemon.failUnpair = new Set();
  daemon.failPair = false;
  daemon.calls = [];
});

afterEach(cleanup);

describe("review transfers", () => {
  it("pairs one candidate as the owner chose it, hides it, and undoes it", async () => {
    const { onDecided } = mount();
    await waitFor(() => expect(rows()).toHaveLength(2));

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Pair Hot a to Cold a" }));
    });

    expect(daemon.calls.find((call) => call.kind === "ui.transfers.pair")?.args).toMatchObject({
      tx_out: "out-a",
      tx_in: "in-a",
      kind: "manual",
      policy: "carrying-value",
      pair_source: "manual",
      confidence_at_pair: "strong",
    });
    expect(onDecided).toHaveBeenCalledTimes(1);
    expect(rows()).toHaveLength(1);
    expect(screen.getByRole("status").textContent).toContain("Paired Hot a to Cold a.");

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    expect(daemon.calls.find((call) => call.kind === "ui.transfers.unpair")?.args).toEqual({
      pair_id: "pair-1",
    });
    // Back in the list at once, before the journals recalculate.
    expect(rows()).toHaveLength(2);
  });

  it("sets a candidate aside and undoes that through the dismissal it made", async () => {
    mount();
    await waitFor(() => expect(rows()).toHaveLength(2));

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Not a transfer: Hot b to Cold b" }));
    });
    expect(rows()).toHaveLength(1);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    expect(
      daemon.calls.find((call) => call.kind === "ui.transfers.dismissals.delete")?.args,
    ).toEqual({ dismissal_id: "dismissal-1" });
  });

  it("keeps the list as read until the recalculation finishes, then reads it again", async () => {
    const view = mount();
    await waitFor(() => expect(rows()).toHaveLength(2));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Pair Hot a to Cold a" }));
    });
    view.rerender(true);
    // A read of stale journals lists more than is open; it is not shown.
    daemon.candidates = [candidate("b"), candidate("c"), candidate("d")];
    expect(rows()).toHaveLength(1);

    daemon.candidates = [candidate("b")];
    view.rerender(false);
    await waitFor(() =>
      expect(daemon.calls.filter((call) => call.kind === "ui.transfers.suggest")).toHaveLength(2),
    );
    await waitFor(() => expect(rows()).toHaveLength(1));
    expect(screen.getByText("Hot b")).toBeTruthy();
  });

  it("pairs exact matches in bulk only after a confirmation, and never ownership proofs", async () => {
    daemon.candidates = [
      candidate("a", { confidence: "exact" }),
      candidate("b", { confidence: "exact" }),
      candidate("c", { confidence: "exact", method: "ownership_graph" }),
      candidate("d", { confidence: "exact", conflict_size: 2 }),
    ];
    mount();
    await waitFor(() => expect(rows()).toHaveLength(4));

    fireEvent.click(screen.getByRole("button", { name: "Pair all 2 exact matches" }));
    expect(daemon.calls.some((call) => call.kind === "ui.transfers.bulk_pair")).toBe(false);
    const dialog = await screen.findByRole("dialog");
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Pair 2" }));
    });

    expect(daemon.calls.find((call) => call.kind === "ui.transfers.bulk_pair")?.args).toEqual({
      confidence: "exact",
    });
    expect(rows()).toHaveLength(2);
    expect(screen.getByRole("status").textContent).toContain("Paired 2 transfers.");
  });

  it("takes the matches that competed for a paired leg off the list, and back on undo", async () => {
    daemon.candidates = [
      candidate("a", { conflict_size: 2 }),
      candidate("x", { out_id: "out-a", conflict_size: 2 }),
      candidate("b"),
    ];
    mount();
    await waitFor(() => expect(rows()).toHaveLength(3));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Pair Hot a to Cold a" }));
    });
    expect(rows()).toHaveLength(1);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    expect(rows()).toHaveLength(3);
  });

  it("keeps what an undo could not reverse undoable, and lists what it did", async () => {
    daemon.candidates = [candidate("a", { confidence: "exact" }), candidate("b", { confidence: "exact" })];
    daemon.failUnpair = new Set(["pair-b"]);
    mount();
    await waitFor(() => expect(rows()).toHaveLength(2));
    fireEvent.click(screen.getByRole("button", { name: "Pair all 2 exact matches" }));
    await act(async () => {
      fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Pair 2" }));
    });
    expect(rows()).toHaveLength(0);

    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Undo" }));
    });
    expect(screen.getByRole("alert").textContent).toContain("That pair changed");
    expect(rows()).toHaveLength(1);
    expect(screen.getByRole("button", { name: "Undo" })).toBeTruthy();
  });

  it("waits with the bulk step while the journals recalculate", async () => {
    daemon.candidates = [candidate("a", { confidence: "exact" })];
    mount({ refreshing: true });
    await waitFor(() => expect(rows()).toHaveLength(1));
    expect(
      (screen.getByRole("button", { name: "Pair 1 exact match" }) as HTMLButtonElement).disabled,
    ).toBe(true);
  });

  it("lists a candidate again when its pairing fails", async () => {
    daemon.failPair = true;
    const { onDecided } = mount();
    await waitFor(() => expect(rows()).toHaveLength(2));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Pair Hot a to Cold a" }));
    });
    expect(rows()).toHaveLength(2);
    expect(screen.getByRole("alert").textContent).toContain("That leg is paired already");
    expect(onDecided).not.toHaveBeenCalled();
  });

  it("renders nothing when there is nothing to decide", async () => {
    daemon.candidates = [];
    mount();
    await waitFor(() =>
      expect(daemon.calls.some((call) => call.kind === "ui.transfers.suggest")).toBe(true),
    );
    expect(screen.queryByRole("heading", { name: /Moves between your wallets/ })).toBeNull();
  });
});
