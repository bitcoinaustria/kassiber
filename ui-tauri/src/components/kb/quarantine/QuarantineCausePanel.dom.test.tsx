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
  onUnpair: (pairIds: string[], onProgress?: (done: number) => void) => Promise<Array<{ pairId: string; message: string }>> = vi.fn(
    async (pairIds: string[], onProgress?: (done: number) => void) => {
      pairIds.forEach((_, index) => onProgress?.(index + 1));
      return [];
    },
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
  it("offers no bulk step for a single pair; its own Unpair is the main action", () => {
    mount(snapshotOf(many(1)));
    expect(screen.queryByRole("button", { name: /Unpair all|Unpair these/ })).toBeNull();
    expect(screen.getByRole("button", { name: "Unpair" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /…$/ })).toBeNull();
  });

  it("covers every pair of the cause in one button, folding a long list", () => {
    mount(snapshotOf(many(8)));
    expect(screen.getByText(/All 8 pairs join two different on-chain transactions/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Unpair all 8 pairs" })).toBeTruthy();
    // With the bulk step there, the pairs carry no buttons of their own.
    expect(screen.queryByRole("button", { name: "Unpair" })).toBeNull();
    // Three shown, the rest one click away; the bulk step still covers all 8.
    expect(screen.getAllByText("Sent")).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "Show all 8 pairs" }));
    expect(screen.getAllByText("Sent")).toHaveLength(8);
    fireEvent.click(screen.getByRole("button", { name: "Show fewer" }));
    expect(screen.getAllByText("Sent")).toHaveLength(3);
  });

  it("opens a pair by clicking it", () => {
    const { onOpenTransaction } = mount(snapshotOf(many(2)));
    fireEvent.click(screen.getAllByText("Sent")[1]);
    expect(onOpenTransaction).toHaveBeenCalledWith("tx-1", "linked", expect.objectContaining({ reason: "custody_quantity_unresolved" }));
  });

  it("bulk-unpairs only the pairs whose evidence decides it", async () => {
    const { onUnpair } = mount(snapshotOf(many(3, (index) => index !== 1)));
    expect(screen.getByText(/2 of these 3 pairs join two different on-chain transactions/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Unpair these 2" }));
    fireEvent.click(await screen.findByRole("button", { name: "Unpair 2 and recalculate" }));
    await waitFor(() => expect(onUnpair).toHaveBeenCalledWith(["pair-0", "pair-2"], expect.any(Function)));
  });

  it("names a few pairs in the confirmation and counts the rest, then recalculates once", async () => {
    const { onUnpair, onProcessJournals } = mount(snapshotOf(many(8)));
    fireEvent.click(screen.getByRole("button", { name: "Unpair all 8 pairs" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("Unpair these 8 pairs?");
    expect(dialog.textContent).toContain("each payment as a disposal");
    expect(dialog.querySelectorAll("li")).toHaveLength(5);
    expect(dialog.textContent).toContain("and 3 more pairs");
    fireEvent.click(screen.getByRole("button", { name: "Unpair 8 and recalculate" }));
    await waitFor(() => expect(onUnpair).toHaveBeenCalledOnce());
    expect((onUnpair as ReturnType<typeof vi.fn>).mock.calls[0][0]).toHaveLength(8);
    await waitFor(() => expect(onProcessJournals).toHaveBeenCalledOnce());
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("says when the cause holds more pairs than this page lists", () => {
    mount(snapshotOf(many(3), 120));
    expect(screen.getByText("117 more pairs of this cause are listed once these are resolved.")).toBeTruthy();
  });

  it("unpairs one undecided pair at a time, naming both wallets in its confirmation", async () => {
    // Without decisive evidence each pair is judged on its own.
    const { onUnpair, onProcessJournals } = mount(snapshotOf(many(2, () => false)));
    expect(screen.queryByRole("button", { name: /Unpair all|Unpair these/ })).toBeNull();
    expect(screen.getAllByRole("button", { name: "Unpair" })).toHaveLength(2);
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair" })[1]);
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("Unpair these two transactions?");
    expect(dialog.textContent).toContain("the payment from Merchant as a disposal");
    expect(dialog.textContent).toContain("the receipt in Spending as a purchase");
    fireEvent.click(screen.getByRole("button", { name: "Unpair and recalculate" }));
    await waitFor(() => expect(onUnpair).toHaveBeenCalledWith(["pair-1"], expect.any(Function)));
    await waitFor(() => expect(onProcessJournals).toHaveBeenCalledOnce());
  });

  it("keeps the failed pairs in the dialog with the reason, and still recalculates the rest", async () => {
    const onUnpair = vi.fn(async () => [{ pairId: "pair-1", message: "pair is locked" }]);
    const { onProcessJournals } = mount(snapshotOf(many(2)), onUnpair);
    fireEvent.click(screen.getByRole("button", { name: "Unpair all 2 pairs" }));
    fireEvent.click(await screen.findByRole("button", { name: "Unpair 2 and recalculate" }));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("1 of 2 could not be unpaired: pair is locked");
    await waitFor(() => expect(onProcessJournals).toHaveBeenCalledOnce());
    // Only the pair that failed is left to retry.
    expect(screen.getByRole("dialog").textContent).toContain("Unpair these two transactions?");
  });

  it("does not recalculate when nothing was unpaired, and Cancel changes nothing", async () => {
    const onUnpair = vi.fn(async (pairIds: string[]) => pairIds.map((pairId) => ({ pairId, message: "offline" })));
    const { onProcessJournals } = mount(snapshotOf(many(2)), onUnpair);
    fireEvent.click(screen.getByRole("button", { name: "Unpair all 2 pairs" }));
    fireEvent.click(await screen.findByRole("button", { name: "Unpair 2 and recalculate" }));
    await screen.findByRole("alert");
    expect(onProcessJournals).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});
