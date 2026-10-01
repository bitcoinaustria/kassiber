// @vitest-environment happy-dom
//
// Mounted: a pair-made suspense is fixed from the page, previewed by the daemon.
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

beforeEach(() => {
  daemon.cases.mockReset().mockResolvedValue({ data: { input_version: 7 } });
  daemon.plan.mockReset().mockImplementation(async (args: { operations: unknown[] }) => ({ data: artifactFor(args) }));
  daemon.apply.mockReset().mockResolvedValue({ data: {} });
});
afterEach(cleanup);

function mount(data: QuarantineSnapshot, onOpenTransaction = vi.fn(), onProcessJournals = vi.fn()) {
  render(
    <QuarantineCausePanel
      snapshot={data}
      isProcessingJournals={false}
      onProcessJournals={onProcessJournals}
      onOpenTransaction={onOpenTransaction}
      onConnectWallet={() => {}}
      onImportHistory={() => {}}
      onShowWaiting={() => {}}
    />,
  );
  return { onOpenTransaction, onProcessJournals };
}

describe("fixing pairs that leave a suspense", () => {
  it("fixes every decided pair with one button, previewed before anything changes", async () => {
    mount(snapshotOf(many(8)));
    expect(screen.getByText(/Kassiber can fix these 8 itself/)).toBeTruthy();
    expect(screen.getByText("Fix above unpairs them, so each side is booked on its own.")).toBeTruthy();
    // Decided pairs carry no buttons of their own, nor repeat why they go.
    expect(screen.queryByText(/The two sides are different on-chain transactions/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Unpair" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Fix all 8" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("Fix these 8 issues?");
    expect(await screen.findByText("In quarantine: 11 → 0")).toBeTruthy();
    expect(screen.getByText("Reports: ready after this")).toBeTruthy();
    expect(daemon.cases).toHaveBeenCalledWith({ limit: 1 });
    const planned = daemon.plan.mock.calls[0][0] as { operations: Array<Record<string, string>>; expected_input_version: number };
    expect(planned.expected_input_version).toBe(7);
    expect(planned.operations).toHaveLength(8);
    expect(planned.operations[0]).toEqual(expect.objectContaining({ type: "unpair", pair_id: "pair-0" }));
    expect(planned.operations[0].reason).toContain("different on-chain txids");
    // A few are named; the rest are counted.
    expect(dialog.querySelectorAll("li")).toHaveLength(5);
    expect(dialog.textContent).toContain("and 3 more pairs");
    expect(daemon.apply).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Fix 8" }));
    await waitFor(() => expect(daemon.apply).toHaveBeenCalledOnce());
    const applied = daemon.apply.mock.calls[0][0] as { artifact: { digest: string }; expected_scope: unknown; idempotency_key: string };
    expect(applied.artifact.digest).toBe("d".repeat(64));
    expect(applied.expected_scope).toEqual({ workspace_id: "ws", profile_id: "book" });
    expect(applied.idempotency_key).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(useUiStore.getState().notifications.some((entry) => entry.title === "Fixed 8 issues")).toBe(true);
  });

  it("offers a single fix plainly", () => {
    mount(snapshotOf(many(1)));
    expect(screen.getByText(/Kassiber can fix this itself/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Fix" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Unpair" })).toBeNull();
  });

  it("fixes the decided pairs and leaves the undecided one to the owner", async () => {
    mount(snapshotOf(many(3, (index) => index !== 1)));
    expect(screen.getByText(/Kassiber can fix 2 of them itself/)).toBeTruthy();
    // One pair still needs comparing, so the card keeps its instructions.
    expect(screen.queryByText(/Fix above unpairs them/)).toBeNull();
    expect(screen.getAllByRole("button", { name: "Unpair" })).toHaveLength(1);
    fireEvent.click(screen.getByRole("button", { name: "Fix 2" }));
    await screen.findByText("In quarantine: 11 → 0");
    const planned = daemon.plan.mock.calls[0][0] as { operations: Array<{ pair_id: string }> };
    expect(planned.operations.map((operation) => operation.pair_id)).toEqual(["pair-0", "pair-2"]);
  });

  it("unpairs an undecided pair the owner picks through the same preview", async () => {
    daemon.plan.mockImplementation(async (args: { operations: unknown[] }) => ({ data: artifactFor(args, 4) }));
    mount(snapshotOf(many(2, () => false)));
    expect(screen.queryByRole("button", { name: /^Fix/ })).toBeNull();
    fireEvent.click(screen.getAllByRole("button", { name: "Unpair" })[1]);
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("Unpair these two transactions?");
    expect(dialog.textContent).toContain("the payment from Merchant as a disposal");
    expect(await screen.findByText("In quarantine: 11 → 4")).toBeTruthy();
    expect(screen.getByText("Reports: still blocked afterwards")).toBeTruthy();
    const planned = daemon.plan.mock.calls[0][0] as { operations: Array<Record<string, string>> };
    expect(planned.operations).toEqual([expect.objectContaining({ type: "unpair", pair_id: "pair-1" })]);
    expect(planned.operations[0].reason).toContain("owner reviewed");
    fireEvent.click(screen.getByRole("button", { name: "Unpair and recalculate" }));
    await waitFor(() => expect(daemon.apply).toHaveBeenCalledOnce());
  });

  it("previews again when the book changed before confirming", async () => {
    daemon.apply.mockRejectedValueOnce(new DaemonRequestError("ui.review.apply", { error: { code: "review_plan_stale" } } as never));
    mount(snapshotOf(many(2)));
    fireEvent.click(screen.getByRole("button", { name: "Fix all 2" }));
    fireEvent.click(await screen.findByRole("button", { name: "Fix 2" }));
    expect(await screen.findByText("The book changed since this check.")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Check again" }));
    await waitFor(() => expect(daemon.plan).toHaveBeenCalledTimes(2));
    fireEvent.click(await screen.findByRole("button", { name: "Fix 2" }));
    await waitFor(() => expect(daemon.apply).toHaveBeenCalledTimes(2));
    // Each preview is confirmed under its own key.
    const keys = daemon.apply.mock.calls.map((call) => (call[0] as { idempotency_key: string }).idempotency_key);
    expect(new Set(keys).size).toBe(2);
  });

  it("shows a failed preview and changes nothing on Cancel", async () => {
    daemon.plan.mockRejectedValueOnce(new Error("daemon offline"));
    mount(snapshotOf(many(2)));
    fireEvent.click(screen.getByRole("button", { name: "Fix all 2" }));
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
    // Not everything is covered, so the page does not claim "all".
    expect(screen.getByRole("button", { name: "Fix 3" })).toBeTruthy();
  });
});
