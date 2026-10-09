// @vitest-environment happy-dom
//
// Mounted: a pair-made suspense is unpaired as the owner picks, previewed by the daemon.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.hoisted(() => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
    removeItem: (key: string) => void storage.delete(key),
  });
});

const daemon = vi.hoisted(() => ({
  cases: vi.fn(),
  plan: vi.fn(),
  apply: vi.fn(),
}));

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));
vi.mock("@/daemon/client", () => {
  class DaemonRequestError extends Error {
    envelope: { error?: { code: string } };
    constructor(_kind: string, envelope: { error?: { code: string } }) {
      super(envelope.error?.code ?? "error");
      this.envelope = envelope;
    }
  }
  const byKind: Record<string, (args: unknown) => Promise<unknown>> = {
    "ui.review.cases": (args) => daemon.cases(args),
    "ui.review.plan": (args) => daemon.plan(args),
    "ui.review.apply": (args) => daemon.apply(args),
  };
  return {
    DaemonRequestError,
    useDaemonStreamMutation: () => ({ mutate: vi.fn(), isPending: false }),
    useDaemonMutation: (kind: string) => ({ mutateAsync: byKind[kind] }),
  };
});

import "@/i18n";
import { DaemonRequestError } from "@/daemon/client";
import { useUiStore } from "@/store/ui";

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
    pair_fingerprint: `fp-${pairId}`,
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
      workspace_id: "ws",
      profile_id: "book",
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
    },
    items: roots,
  };
}

const many = (count: number, differ: (index: number) => boolean = () => true) =>
  Array.from({ length: count }, (_, index) => root(`tx-${index}`, `pair-${index}`, differ(index)));

/** The daemon's preview: what the operations change, computed on a copy. */
function artifactFor(args: { operations: unknown[] }, after = 0) {
  return {
    schema_version: 1,
    workspace_id: "ws",
    profile_id: "book",
    base_input_version: 7,
    digest: "d".repeat(64),
    operations: args.operations,
    before: { entries_count: 10, quarantine_count: 11, report_ready: false, quarantines: [] },
    after: { entries_count: 12, quarantine_count: after, report_ready: after === 0, quarantines: [] },
  };
}

// What a fresh read of the attention page returns; the snapshot by default.
const fresh = vi.hoisted(() => ({
  items: null as QuarantineItem[] | null,
  scope: null as { workspace_id: string; profile_id: string } | null,
}));

beforeEach(() => {
  fresh.items = null;
  fresh.scope = null;
  daemon.cases.mockReset().mockResolvedValue({ data: { input_version: 7 } });
  daemon.plan.mockReset().mockImplementation(async (args: { operations: unknown[] }) => ({ data: artifactFor(args) }));
  daemon.apply.mockReset().mockResolvedValue({ data: {} });
});
afterEach(cleanup);

const BOOK = { workspace_id: "ws", profile_id: "book" };

function mount(data: QuarantineSnapshot, onOpenTransaction = vi.fn(), onProcessJournals = vi.fn()) {
  const onRefresh = vi.fn(async () => ({ items: fresh.items ?? data.items, scope: fresh.scope ?? BOOK }));
  const panel = (snapshot: QuarantineSnapshot) => (
    <QuarantineCausePanel
      snapshot={snapshot}
      isProcessingJournals={false}
      onProcessJournals={onProcessJournals}
      onOpenTransaction={onOpenTransaction}
      onConnectWallet={() => {}}
      onImportHistory={() => {}}
      onShowWaiting={() => {}}
      onRefresh={onRefresh}
    />
  );
  const view = render(panel(data));
  // A later read of the same page, as a sync or another session leaves it.
  const reread = (snapshot: QuarantineSnapshot) => view.rerender(panel(snapshot));
  return { onOpenTransaction, onProcessJournals, onRefresh, reread };
}

const planned = (call = 0) =>
  daemon.plan.mock.calls[call][0] as { operations: Array<Record<string, string>>; expected_input_version: number };

describe("unpairing pairs that leave a suspense", () => {
  it("picks nothing by itself: no Fix all, every pair keeps its own Unpair", () => {
    // Every pair joins two different txids; still none is chosen for the owner.
    mount(snapshotOf(many(8)));
    expect(screen.queryByRole("button", { name: /^Fix/ })).toBeNull();
    expect(screen.queryByText(/Kassiber can fix/)).toBeNull();
    expect(screen.getAllByRole("button", { name: "Unpair" })).toHaveLength(3);
    for (const box of screen.getAllByRole("checkbox")) {
      expect(box.getAttribute("aria-checked")).toBe("false");
    }
    expect(screen.queryByRole("button", { name: /picked/ })).toBeNull();
    expect(screen.getAllByText(/this alone doesn't make the pair wrong/).length).toBeGreaterThan(0);
  });

  it("leaves a legitimate hop through an untracked wallet alone unless the owner picks it", async () => {
    // Sent 1 BTC in one transaction, got 0.99 BTC back in another: two txids,
    // one movement. Unpairing the pair next to it does not touch it.
    const hop = root("hop", "pair-hop");
    mount(snapshotOf([hop, ...many(2)]));
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair" })[1]);
    await screen.findByText("In quarantine: 11 → 0");
    expect(planned().operations.map((operation) => operation.pair_id)).toEqual(["pair-0"]);
    expect(daemon.apply).not.toHaveBeenCalled();
  });

  it("unpairs the pairs the owner ticked in one previewed step", async () => {
    mount(snapshotOf(many(8)));
    const boxes = screen.getAllByRole("checkbox");
    fireEvent.click(boxes[0]);
    fireEvent.click(boxes[2]);
    fireEvent.click(screen.getByRole("button", { name: "Unpair 2 picked pairs" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("Unpair these 2 pairs?");
    expect(dialog.textContent).toContain("You picked these 2 pairs");
    expect(await screen.findByText("In quarantine: 11 → 0")).toBeTruthy();
    expect(daemon.cases).toHaveBeenCalledWith({ limit: 1, expected_scope: BOOK });
    expect(planned().expected_input_version).toBe(7);
    expect(planned().operations.map((operation) => operation.pair_id)).toEqual(["pair-0", "pair-2"]);
    // Bound to this book and to each pair as it read when ticked.
    expect((daemon.plan.mock.calls[0][0] as { expected_scope: unknown }).expected_scope).toEqual(BOOK);
    expect(planned().operations.map((operation) => operation.expected_fingerprint)).toEqual(["fp-pair-0", "fp-pair-2"]);
    // The audit reason records the owner's choice, not a verdict on the txids.
    expect(planned().operations[0].reason).toContain("owner");
    expect(planned().operations[0].reason).not.toMatch(/txid/);
    fireEvent.click(screen.getByRole("button", { name: "Unpair 2 and recalculate" }));
    await waitFor(() => expect(daemon.apply).toHaveBeenCalledOnce());
    const applied = daemon.apply.mock.calls[0][0] as { artifact: { digest: string }; expected_scope: unknown; idempotency_key: string };
    expect(applied.artifact.digest).toBe("d".repeat(64));
    expect(applied.expected_scope).toEqual({ workspace_id: "ws", profile_id: "book" });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(useUiStore.getState().notifications.some((entry) => entry.title === "Unpaired 2 pairs")).toBe(true);
  });

  it("unpairs one pair through the same preview, naming both wallets", async () => {
    daemon.plan.mockImplementation(async (args: { operations: unknown[] }) => ({ data: artifactFor(args, 4) }));
    mount(snapshotOf(many(2, () => false)));
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair" })[1]);
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("Unpair these two transactions?");
    expect(dialog.textContent).toContain("the payment from Merchant as a disposal");
    expect(await screen.findByText("In quarantine: 11 → 4")).toBeTruthy();
    expect(screen.getByText("Reports: still blocked afterwards")).toBeTruthy();
    expect(planned().operations).toEqual([expect.objectContaining({ type: "unpair", pair_id: "pair-1" })]);
    fireEvent.click(screen.getByRole("button", { name: "Unpair and recalculate" }));
    await waitFor(() => expect(daemon.apply).toHaveBeenCalledOnce());
  });

  it("checks again against fresh data: a pair revised meanwhile is not replanned", async () => {
    daemon.apply.mockRejectedValueOnce(new DaemonRequestError("ui.review.apply", { error: { code: "review_plan_stale" } } as never));
    const pairs = many(2);
    mount(snapshotOf(pairs));
    fireEvent.click(screen.getAllByRole("checkbox")[0]);
    fireEvent.click(screen.getAllByRole("checkbox")[1]);
    fireEvent.click(screen.getByRole("button", { name: "Unpair 2 picked pairs" }));
    fireEvent.click(await screen.findByRole("button", { name: "Unpair 2 and recalculate" }));
    expect(await screen.findByText("The book changed since this check.")).toBeTruthy();
    // Another session turned the first pair into a reviewed swap refund.
    const revised = {
      ...pairs[0],
      evidence: { ...pairs[0].evidence, pair_review: { kind: "swap_refund", policy: null, out_amount_msat: 1, in_amount_msat: 1 } },
    };
    fresh.items = [revised, pairs[1]];
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    expect((await screen.findByRole("alert")).textContent).toContain("1 pair you picked changed since you saw it");
    expect(daemon.plan).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: /and recalculate/ })).toBeNull();
  });

  it("checks again against fresh data: a pair that cleared meanwhile is left out", async () => {
    daemon.apply.mockRejectedValueOnce(new DaemonRequestError("ui.review.apply", { error: { code: "review_plan_stale" } } as never));
    const pairs = many(2);
    mount(snapshotOf(pairs));
    fireEvent.click(screen.getAllByRole("checkbox")[0]);
    fireEvent.click(screen.getAllByRole("checkbox")[1]);
    fireEvent.click(screen.getByRole("button", { name: "Unpair 2 picked pairs" }));
    fireEvent.click(await screen.findByRole("button", { name: "Unpair 2 and recalculate" }));
    await screen.findByText("The book changed since this check.");
    fresh.items = [pairs[1]];
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    expect(await screen.findByText(/1 pair you picked is no longer listed/)).toBeTruthy();
    expect(planned(1).operations.map((operation) => operation.pair_id)).toEqual(["pair-1"]);
  });

  it("retries an apply that failed without an answer with the same proposal and key", async () => {
    daemon.apply.mockRejectedValueOnce(new Error("connection lost"));
    mount(snapshotOf(many(2)));
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair" })[0]);
    fireEvent.click(await screen.findByRole("button", { name: "Unpair and recalculate" }));
    expect((await screen.findByRole("alert")).textContent).toContain("connection lost");
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    await waitFor(() => expect(daemon.apply).toHaveBeenCalledTimes(2));
    expect(daemon.plan).toHaveBeenCalledOnce();
    const keys = daemon.apply.mock.calls.map((call) => (call[0] as { idempotency_key: string }).idempotency_key);
    expect(new Set(keys).size).toBe(1);
  });

  it("shows a failed preview and changes nothing on Cancel", async () => {
    daemon.plan.mockRejectedValueOnce(new Error("daemon offline"));
    mount(snapshotOf(many(2)));
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair" })[0]);
    expect((await screen.findByRole("alert")).textContent).toContain("daemon offline");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(daemon.apply).not.toHaveBeenCalled();
  });

  it("folds a long list of pairs", () => {
    mount(snapshotOf(many(8)));
    expect(screen.getAllByText("Sent")).toHaveLength(3);
    fireEvent.click(screen.getByRole("button", { name: "Show all 8 pairs" }));
    expect(screen.getAllByText("Sent")).toHaveLength(8);
    expect(screen.getAllByRole("button", { name: "Unpair" })).toHaveLength(8);
    fireEvent.click(screen.getByRole("button", { name: "Show fewer" }));
    expect(screen.getAllByText("Sent")).toHaveLength(3);
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

  it("drops a tick when its row now holds another pair, never moving it to the new one", async () => {
    const pairs = many(3);
    const { reread } = mount(snapshotOf(pairs));
    fireEvent.click(screen.getAllByRole("checkbox")[0]);
    expect(screen.getByRole("button", { name: "Unpair the picked pair" })).toBeTruthy();
    // Another session replaced P1 (pair-0) with P2 on the same transaction.
    const replaced = {
      ...pairs[0],
      evidence: { ...pairs[0].evidence, pair_id: "pair-new", pair_fingerprint: "fp-pair-new" },
    };
    reread(snapshotOf([replaced, pairs[1], pairs[2]]));
    expect(screen.getAllByRole("checkbox")[0].getAttribute("aria-checked")).toBe("false");
    expect(screen.queryByRole("button", { name: /picked pair/ })).toBeNull();
    expect(screen.getByRole("status").textContent).toContain("1 pair you ticked changed or cleared since");
    expect(daemon.plan).not.toHaveBeenCalled();
  });

  it("drops a tick when the same pair id was revised since", async () => {
    const pairs = many(3);
    const { reread } = mount(snapshotOf(pairs));
    fireEvent.click(screen.getAllByRole("checkbox")[0]);
    fireEvent.click(screen.getAllByRole("checkbox")[1]);
    // pair-0 keeps its id but its kind or amounts changed: a new reading.
    const revised = { ...pairs[0], evidence: { ...pairs[0].evidence, pair_fingerprint: "fp-pair-0-revised" } };
    reread(snapshotOf([revised, pairs[1], pairs[2]]));
    expect(screen.getAllByRole("checkbox")[0].getAttribute("aria-checked")).toBe("false");
    fireEvent.click(screen.getByRole("button", { name: "Unpair the picked pair" }));
    await screen.findByText("In quarantine: 11 → 0");
    expect(planned().operations.map((operation) => operation.pair_id)).toEqual(["pair-1"]);
  });

  it("stops when another book is open by the time it checks", async () => {
    mount(snapshotOf(many(2)));
    fresh.scope = { workspace_id: "ws", profile_id: "other-book" };
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair" })[0]);
    expect((await screen.findByRole("alert")).textContent).toContain("Another book is open now");
    expect(daemon.plan).not.toHaveBeenCalled();
  });
});
