// @vitest-environment happy-dom
//
// Mounted: a pair-made suspense is resolved from its card.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("@/daemon/client", () => ({
  useDaemonStreamMutation: () => ({ mutate: vi.fn(), isPending: false }),
}));

import "@/i18n";

import { QuarantineCausePanel } from "./QuarantineCausePanel";
import type { QuarantineEvidence, QuarantineItem, QuarantineSnapshot } from "./types";

const legs = (out: string, inn: string) => ({
  out: {
    transaction_id: out,
    wallet: "Merchant",
    asset: "BTC",
    amount_msat: 15_025_943_000,
    occurred_at: "2023-02-01T04:41:17Z",
    external_id: "a".repeat(64),
  },
  in: {
    transaction_id: inn,
    wallet: "Spending",
    asset: "BTC",
    amount_msat: 14_964_523_000,
    occurred_at: "2023-02-01T02:15:00Z",
    external_id: "b".repeat(64),
  },
});

const GROUP_KEY = "custody_quantity_unresolved:reviewed_residual_suspense:";

function root(id: string, pairId: string, txidsDiffer = true): QuarantineItem {
  const evidence: QuarantineEvidence = {
    blocker_code: "reviewed_residual_suspense",
    pair_id: pairId,
    pair_counterpart_transaction_id: `${id}-in`,
    pair_txids_differ: txidsDiffer,
    pair_receipt_before_spend: true,
    pair_legs: legs(id, `${id}-in`),
  };
  return {
    transaction_id: id,
    external_id: "a".repeat(64),
    occurred_at: "2023-02-01T04:41:17Z",
    confirmed_at: null,
    wallet: "Merchant",
    direction: "outbound",
    asset: "BTC",
    amount: 0.15,
    amount_msat: 15_025_943_000,
    fee: 0,
    fee_msat: 0,
    reason: "custody_quantity_unresolved",
    detail: {},
    created_at: "2026-09-30T19:38:23Z",
    category: "needs_decision",
    blocks_reports: true,
    is_downstream: false,
    root: null,
    evidence,
    actions: [{ kind: "review_pair", transaction_id: id, pair_id: pairId }],
    group_key: GROUP_KEY,
  };
}

function snapshotOf(roots: QuarantineItem[], rootCount = roots.length): QuarantineSnapshot {
  return {
    summary: {
      workspace: "Books",
      profile: "Book",
      count: rootCount + 3,
      by_reason: [],
      limit: 100,
      offset: 0,
      blocking_count: rootCount,
      reports_blocked: true,
      freshness: { needs_processing: false, last_processed_at: "2026-09-30T19:38:23Z", last_error: null },
      groups: [
        {
          key: GROUP_KEY,
          category: "needs_decision",
          reason: "custody_quantity_unresolved",
          root_transaction_id: roots[0].transaction_id,
          root_occurred_at: "2023-02-01T04:41:17Z",
          root_wallet: "Merchant",
          root_external_id: "a".repeat(64),
          root_amount_msat: 15_025_943_000,
          root_direction: "outbound",
          root_asset: "BTC",
          count: rootCount + 3,
          downstream_count: 3,
          blocks_reports: true,
          wallets: ["Merchant"],
          earliest_occurred_at: "2023-02-01T04:41:17Z",
          evidence: roots[0].evidence!,
          actions: roots[0].actions!,
          // The summary names at most a few roots; the cause holds more.
          root_transaction_ids: roots.slice(0, 2).map((item) => item.transaction_id),
          root_count: rootCount,
        },
      ],
      group_count: 1,
      attention_count: rootCount,
      waiting_count: 3,
      scope: "attention",
      scope_count: rootCount,
      assumptions: null,
    },
    items: roots,
  };
}

const many = (count: number, differ: (index: number) => boolean = () => true) =>
  Array.from({ length: count }, (_, index) => root(`tx-${index}`, `pair-${index}`, differ(index)));

afterEach(cleanup);

function mount(
  data: QuarantineSnapshot,
  onUnpair: (item: QuarantineItem) => Promise<"unpaired" | "changed"> = vi.fn(
    async () => "unpaired" as const,
  ),
  onProcessJournals = vi.fn(),
  onOpenTransaction = vi.fn(),
) {
  render(
    <QuarantineCausePanel
      snapshot={data}
      isProcessingJournals={false}
      onProcessJournals={onProcessJournals}
      onOpenTransaction={onOpenTransaction}
      onConnectWallet={() => {}}
      onImportHistory={() => {}}
      onShowWaiting={() => {}}
      onUnpair={onUnpair}
    />,
  );
  return { onUnpair, onProcessJournals, onOpenTransaction };
}

describe("resolving pairs that leave a suspense", () => {
  it("gives every pair its own Unpair and no bulk step", () => {
    mount(snapshotOf(many(8)));
    expect(screen.queryByRole("button", { name: /Unpair all|Unpair these/ })).toBeNull();
    // Three shown, the rest one click away; each one keeps its own Unpair.
    expect(screen.getAllByText("Sent")).toHaveLength(3);
    expect(screen.getAllByRole("button", { name: "Unpair" })).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "Show all 8 pairs" }));
    expect(screen.getAllByText("Sent")).toHaveLength(8);
    expect(screen.getAllByRole("button", { name: "Unpair" })).toHaveLength(8);
    fireEvent.click(screen.getByRole("button", { name: "Show fewer" }));
    expect(screen.getAllByText("Sent")).toHaveLength(3);
  });

  it("shows different txids as a hint and never unpairs a legitimate hop on its own", async () => {
    // Sent 1 BTC in one transaction and got 0.99 BTC back in another, through
    // a wallet the book does not track: two txids, still one movement.
    const hop = root("hop", "pair-hop");
    const { onUnpair, onProcessJournals } = mount(snapshotOf([hop, ...many(2)]));
    expect(screen.getAllByText(/this alone doesn't make the pair wrong/).length).toBeGreaterThan(0);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(screen.queryByRole("checkbox")).toBeNull();
    // Unpairing another pair touches only that pair.
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair" })[2]);
    fireEvent.click(await screen.findByRole("button", { name: "Unpair and recalculate" }));
    await waitFor(() => expect(onUnpair).toHaveBeenCalledOnce());
    expect((onUnpair as ReturnType<typeof vi.fn>).mock.calls[0][0].evidence.pair_id).toBe("pair-1");
    await waitFor(() => expect(onProcessJournals).toHaveBeenCalledOnce());
  });

  it("opens a pair by clicking it", () => {
    const { onOpenTransaction } = mount(snapshotOf(many(2)));
    fireEvent.click(screen.getAllByText("Sent")[1]);
    expect(onOpenTransaction).toHaveBeenCalledWith("tx-1", "linked", expect.objectContaining({ reason: "custody_quantity_unresolved" }));
  });

  it("says when the cause holds more pairs than this page lists", () => {
    mount(snapshotOf(many(3), 120));
    expect(screen.getByText("117 more pairs of this cause are listed once these are resolved.")).toBeTruthy();
  });

  it("unpairs one pair at a time, naming both wallets in its confirmation", async () => {
    const { onUnpair, onProcessJournals } = mount(snapshotOf(many(2, () => false)));
    expect(screen.getAllByRole("button", { name: "Unpair" })).toHaveLength(2);
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair" })[1]);
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("Unpair these two transactions?");
    expect(dialog.textContent).toContain("the payment from Merchant as a disposal");
    expect(dialog.textContent).toContain("the receipt in Spending as a purchase");
    fireEvent.click(screen.getByRole("button", { name: "Unpair and recalculate" }));
    await waitFor(() => expect(onUnpair).toHaveBeenCalledOnce());
    expect((onUnpair as ReturnType<typeof vi.fn>).mock.calls[0][0].transaction_id).toBe("tx-1");
    await waitFor(() => expect(onProcessJournals).toHaveBeenCalledOnce());
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("changes nothing when the pair changed since it was listed", async () => {
    const onUnpair = vi.fn(async () => "changed" as const);
    const { onProcessJournals } = mount(snapshotOf(many(2)), onUnpair);
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair" })[0]);
    fireEvent.click(await screen.findByRole("button", { name: "Unpair and recalculate" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("This pair changed since it was listed");
    expect(onProcessJournals).not.toHaveBeenCalled();
    // It is looked at again first: no second confirmation from this dialog.
    expect(screen.queryByRole("button", { name: "Unpair and recalculate" })).toBeNull();
    // The footer's Close and the dialog's own X both close it.
    fireEvent.click(screen.getAllByRole("button", { name: "Close" })[0]);
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("keeps the dialog with the reason when unpairing fails, and does not recalculate", async () => {
    const onUnpair = vi.fn(async () => {
      throw new Error("pair is locked");
    });
    const { onProcessJournals } = mount(snapshotOf(many(2)), onUnpair);
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair" })[0]);
    fireEvent.click(await screen.findByRole("button", { name: "Unpair and recalculate" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("The pair could not be unpaired: pair is locked");
    expect(onProcessJournals).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});
