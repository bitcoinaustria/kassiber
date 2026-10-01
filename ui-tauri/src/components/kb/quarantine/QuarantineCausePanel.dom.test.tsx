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

function root(id: string, pairId: string): QuarantineItem {
  const evidence: QuarantineEvidence = {
    blocker_code: "reviewed_residual_suspense",
    pair_id: pairId,
    pair_counterpart_transaction_id: `${id}-in`,
    pair_txids_differ: true,
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
  };
}

const ROOTS = [root("rent", "pair-1"), root("payroll", "pair-2")];

const snapshot: QuarantineSnapshot = {
  summary: {
    workspace: "Books",
    profile: "Book",
    count: 5,
    by_reason: [],
    limit: 100,
    offset: 0,
    blocking_count: 2,
    reports_blocked: true,
    freshness: { needs_processing: false, last_processed_at: "2026-09-30T19:38:23Z", last_error: null },
    groups: [
      {
        key: "custody_quantity_unresolved:reviewed_residual_suspense:",
        category: "needs_decision",
        reason: "custody_quantity_unresolved",
        root_transaction_id: "rent",
        root_occurred_at: "2023-02-01T04:41:17Z",
        root_wallet: "Merchant",
        root_external_id: "a".repeat(64),
        root_amount_msat: 15_025_943_000,
        root_direction: "outbound",
        root_asset: "BTC",
        count: 5,
        downstream_count: 3,
        blocks_reports: true,
        wallets: ["Merchant"],
        earliest_occurred_at: "2023-02-01T04:41:17Z",
        evidence: ROOTS[0].evidence!,
        actions: ROOTS[0].actions!,
        root_transaction_ids: ["rent", "payroll"],
        root_count: 2,
      },
    ],
    group_count: 1,
    attention_count: 2,
    waiting_count: 3,
    scope: "attention",
    scope_count: 2,
    assumptions: null,
  },
  items: ROOTS,
};

afterEach(cleanup);

function mount(onUnpair = vi.fn(async () => undefined), onProcessJournals = vi.fn()) {
  render(
    <QuarantineCausePanel
      snapshot={snapshot}
      isProcessingJournals={false}
      onProcessJournals={onProcessJournals}
      onOpenTransaction={vi.fn()}
      onConnectWallet={() => {}}
      onImportHistory={() => {}}
      onShowWaiting={() => {}}
      onUnpair={onUnpair}
    />,
  );
  return { onUnpair, onProcessJournals };
}

describe("resolving a pair that leaves a suspense", () => {
  it("shows every pair side by side with its own way out", () => {
    mount();
    expect(screen.getByText("The 2 pairs")).toBeTruthy();
    expect(screen.getAllByText("Sent")).toHaveLength(2);
    expect(screen.getAllByText("2023-02-01 02:15 · Spending")).toHaveLength(2);
    expect(screen.getAllByRole("button", { name: "Unpair…" })).toHaveLength(2);
    // The pair list replaces the card-wide "Review the pair".
    expect(screen.queryByRole("button", { name: "Review the pair" })).toBeNull();
  });

  it("unpairs only after the owner confirms what that books, then recalculates", async () => {
    const { onUnpair, onProcessJournals } = mount();
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair…" })[1]);
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("Unpair these two transactions?");
    expect(dialog.textContent).toContain("the payment from Merchant as a disposal");
    expect(dialog.textContent).toContain("the receipt in Spending as a purchase");
    expect(onUnpair).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Unpair and recalculate" }));
    await waitFor(() => expect(onUnpair).toHaveBeenCalledWith("pair-2"));
    await waitFor(() => expect(onProcessJournals).toHaveBeenCalledOnce());
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("keeps the dialog open and says why when unpairing fails", async () => {
    const onUnpair = vi.fn(async () => {
      throw new Error("pair is locked");
    });
    const { onProcessJournals } = mount(onUnpair);
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair…" })[0]);
    fireEvent.click(await screen.findByRole("button", { name: "Unpair and recalculate" }));
    expect((await screen.findByRole("alert")).textContent).toContain("pair is locked");
    expect(onProcessJournals).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeTruthy();
  });

  it("closes without changes on Cancel", async () => {
    const { onUnpair } = mount();
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair…" })[0]);
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(onUnpair).not.toHaveBeenCalled();
  });
});
