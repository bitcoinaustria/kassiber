import { describe, expect, it } from "vitest";

import i18n from "@/i18n";

import {
  quarantineItemToRow,
  quarantineMetrics,
  quarantineReasonFilterIds,
  quarantineResolvePlan,
  quarantineRows,
} from "./model";
import type { QuarantineItem, QuarantineSnapshot } from "./types";

const t = i18n.getFixedT("en", "journals");

const baseItem: QuarantineItem = {
  transaction_id: "transaction-with-a-long-id",
  external_id: "",
  occurred_at: "2026-05-01T10:00:00Z",
  confirmed_at: null,
  wallet: "Cold",
  direction: "outbound",
  asset: "BTC",
  amount: 0.01,
  amount_msat: 1_000_000_000,
  fee: 0,
  fee_msat: 0,
  reason: "missing_spot_price",
  detail: {},
  created_at: "2026-05-01T10:01:00Z",
};

describe("quarantine row model", () => {
  it("maps missing price quarantines into review-table rows", () => {
    const row = quarantineItemToRow(baseItem, "AT profile", t);

    expect(row).toMatchObject({
      id: "transactio...",
      date: "2026-05-01",
      account: "Cold",
      event: "Missing BTC price for outbound",
      source: "Journal quarantine · outbound",
      amount: "-1,000,000 sats BTC",
      basis: "Missing fiat price",
      impact: "Held from reports",
      status: "Needs review",
      priority: "Medium",
      owner: "AT profile",
      evidenceHint: "Add a fiat price or rates coverage",
      nextAction: "Set price, then process journals",
      metricFilterIds: ["missing-prices"],
      transactionAction: {
        transactionId: "transaction-with-a-long-id",
        label: "Add price",
        tab: "pricing",
      },
    });
  });

  it("classifies hard basis issues as blocked rows", () => {
    const row = quarantineItemToRow(
      {
        ...baseItem,
        external_id: "tx-123",
        direction: "inbound",
        amount_msat: 250_000_000,
        reason: "insufficient_lots",
      },
      null,
      t,
    );

    expect(row.status).toBe("Blocked");
    expect(row.priority).toBe("High");
    expect(row.owner).toBe("Active book");
    expect(row.event).toBe("Not enough BTC lots in Cold");
    expect(row.amount).toBe("250,000 sats BTC");
    expect(row.metricFilterIds).toEqual(["basis-or-pairs"]);
    expect(row.transactionAction).toMatchObject({
      transactionId: "transaction-with-a-long-id",
      label: "Fix cost basis",
      tab: "tax",
    });
  });

  it("treats coarse pricing review as a non-blocking pricing task", () => {
    const row = quarantineItemToRow(
      {
        ...baseItem,
        reason: "pricing_review_required",
        detail: {
          pricing_quality: "coarse_fallback",
          pricing_granularity: "daily",
          wallet: "Cold",
        },
      },
      "AT profile",
      t,
    );

    expect(row).toMatchObject({
      event: "Coarse BTC price needs review (daily)",
      basis: "Coarse pricing — verify",
      status: "Needs review",
      priority: "Medium",
      evidenceHint: "coarse_fallback / daily price on Cold",
      nextAction: "Fetch precise prices or confirm, then process journals",
      metricFilterIds: ["missing-prices"],
      transactionAction: {
        label: "Add price",
        tab: "pricing",
      },
    });
  });

  it("describes implausible transfer fees as split-transfer review", () => {
    const row = quarantineItemToRow(
      {
        ...baseItem,
        reason: "transfer_fee_implausible",
        detail: {
          from_wallet: "Cold",
          to_wallet: "Hot",
          implied_fee: 0.01952253,
          fee_ceiling: 0.000025,
        },
      },
      "AT profile",
      t,
    );

    expect(row).toMatchObject({
      event: "Implausible transfer fee: Cold -> Hot",
      basis: "Split transfer / swap review",
      evidenceHint: "Implied fee 1,952,253 sats exceeds review ceiling 2,500 sats",
      nextAction: "Review split transfer/swap, then process journals",
      transactionAction: {
        label: "Review details",
        tab: "details",
      },
    });
  });

  it("guides ownership-derived self-transfers that could not be auto-resolved", () => {
    const row = quarantineItemToRow(
      {
        ...baseItem,
        reason: "ownership_transfer_destination_ambiguous",
      },
      "AT profile",
      t,
    );

    expect(row).toMatchObject({
      event: "Self-transfer needs pairing in Cold",
      basis: "Self-transfer between your wallets",
      status: "Needs review",
      priority: "Medium",
      evidenceHint:
        "Proven from the on-chain transaction — an output paid another of your wallets, but the matching receipt couldn't be resolved automatically",
      nextAction:
        "Pair it manually, or sync the destination wallet so its receipt is recorded, then process journals",
      metricFilterIds: ["basis-or-pairs"],
      transactionAction: {
        label: "Review transfer",
        tab: "details",
      },
    });
  });

  it("gives multi-source consolidations their own next action", () => {
    const row = quarantineItemToRow(
      { ...baseItem, reason: "ownership_transfer_source_ambiguous" },
      "AT profile",
      t,
    );
    expect(row.basis).toBe("Self-transfer between your wallets");
    expect(row.nextAction).toBe(
      "Several of your wallets funded this spend — review the consolidation and pair it, then process journals",
    );
  });

  it("tells the user to re-sync on an ownership amount mismatch", () => {
    const row = quarantineItemToRow(
      { ...baseItem, reason: "ownership_transfer_amount_mismatch" },
      "AT profile",
      t,
    );
    expect(row.nextAction).toBe(
      "The recorded amount disagrees with the on-chain transaction — re-sync the wallet, then process journals",
    );
  });
});

describe("quarantine metrics", () => {
  it("groups daemon reason strings into review filters", () => {
    const metrics = quarantineMetrics(
      {
        workspace: "Personal",
        profile: "AT profile",
        count: 7,
        limit: 100,
        by_reason: [
          { reason: "missing_spot_price", count: 2 },
          { reason: "transfer_mismatch", count: 1 },
          { reason: "insufficient_lots", count: 3 },
          { reason: "unsupported_tax_direction", count: 1 },
        ],
      },
      t,
    );

    expect(metrics.map((metric) => [metric.label, metric.value])).toEqual([
      ["Quarantined", 7],
      ["Missing prices", 2],
      ["Basis or pairs", 4],
      ["Other review", 1],
    ]);
  });

  it("folds coarse pricing review into the pricing band", () => {
    const metrics = quarantineMetrics({
      workspace: "Personal",
      profile: "AT profile",
      count: 6,
      limit: 100,
      by_reason: [
        { reason: "missing_spot_price", count: 2 },
        { reason: "pricing_review_required", count: 3 },
        { reason: "insufficient_lots", count: 1 },
      ],
    }, t);

    expect(metrics.map((metric) => [metric.label, metric.value])).toEqual([
      ["Quarantined", 6],
      ["Missing prices", 5],
      ["Basis or pairs", 1],
      ["Other review", 0],
    ]);
  });
});

describe("quarantine resolve plan", () => {
  function snapshotWith(items: QuarantineItem[]): QuarantineSnapshot {
    const byReason = new Map<string, number>();
    for (const item of items) {
      byReason.set(item.reason, (byReason.get(item.reason) ?? 0) + 1);
    }
    return {
      summary: {
        workspace: "Personal",
        profile: "AT profile",
        count: items.length,
        limit: 100,
        by_reason: Array.from(byReason, ([reason, count]) => ({ reason, count })),
      },
      items,
    };
  }

  it("orders repair steps from sync safety through journal processing", () => {
    const snapshot = snapshotWith([
      { ...baseItem, transaction_id: "price-1", reason: "missing_spot_price" },
      { ...baseItem, transaction_id: "basis-1", reason: "insufficient_lots" },
      {
        ...baseItem,
        transaction_id: "transfer-1",
        reason: "ownership_transfer_destination_ambiguous",
      },
      {
        ...baseItem,
        transaction_id: "sync-1",
        reason: "ownership_transfer_amount_mismatch",
      },
      { ...baseItem, transaction_id: "other-1", reason: "unsupported_asset" },
    ]);
    const rows = quarantineRows(snapshot, t);

    const plan = quarantineResolvePlan(snapshot, rows, t);

    expect(plan.total).toBe(5);
    expect(plan.blockedCount).toBe(2);
    expect(plan.actionableCount).toBe(5);
    expect(plan.steps.map((step) => step.id)).toEqual([
      "sync-wallets",
      "review-transfers",
      "fix-basis",
      "add-prices",
      "review-other",
      "process-journals",
    ]);
    expect(plan.summary).toBe(
      "5 quarantined rows · 5 repair steps before journal processing",
    );
  });

  it("attaches the first actionable row to each repair category", () => {
    const snapshot = snapshotWith([
      { ...baseItem, transaction_id: "price-1", reason: "missing_spot_price" },
      {
        ...baseItem,
        transaction_id: "price-2",
        reason: "pricing_review_required",
      },
      { ...baseItem, transaction_id: "basis-1", reason: "insufficient_lots" },
    ]);
    const rows = quarantineRows(snapshot, t);

    const plan = quarantineResolvePlan(snapshot, rows, t);
    const priceStep = plan.steps.find((step) => step.id === "add-prices");
    const basisStep = plan.steps.find((step) => step.id === "fix-basis");
    const processStep = plan.steps.find((step) => step.id === "process-journals");

    expect(priceStep).toMatchObject({
      count: 2,
      title: "Add or confirm prices",
      actionKind: "open-row",
      actionLabel: "Add first price",
      rowIds: ["price-1", "price-2"],
      primaryAction: { transactionId: "price-1", tab: "pricing" },
    });
    expect(priceStep?.previewRows).toHaveLength(2);
    expect(basisStep).toMatchObject({
      count: 1,
      tone: "alert",
      primaryAction: { transactionId: "basis-1", tab: "tax" },
    });
    expect(processStep).toMatchObject({
      count: 3,
      actionKind: "process-journals",
      actionLabel: "Process journals",
    });
    expect(processStep?.primaryAction).toBeUndefined();
  });

  it("returns an empty plan for a clear queue", () => {
    const snapshot = snapshotWith([]);

    const plan = quarantineResolvePlan(snapshot, [], t);

    expect(plan).toMatchObject({
      total: 0,
      summary: "No quarantined rows need repair.",
      steps: [],
      actionableCount: 0,
      blockedCount: 0,
    });
  });

  it("keeps items on distinct repair steps when they share a transaction id", () => {
    const snapshot = snapshotWith([
      { ...baseItem, transaction_id: "shared-tx", reason: "missing_spot_price" },
      { ...baseItem, transaction_id: "shared-tx", reason: "insufficient_lots" },
    ]);
    const rows = quarantineRows(snapshot, t);

    const plan = quarantineResolvePlan(snapshot, rows, t);
    const priceStep = plan.steps.find((step) => step.id === "add-prices");
    const basisStep = plan.steps.find((step) => step.id === "fix-basis");

    expect(priceStep?.count).toBe(1);
    expect(basisStep?.count).toBe(1);
    expect(priceStep?.primaryAction).toMatchObject({
      transactionId: "shared-tx",
      tab: "pricing",
    });
    expect(basisStep?.primaryAction).toMatchObject({
      transactionId: "shared-tx",
      tab: "tax",
    });
    // Each step must preview its own row, not a collapsed duplicate of the
    // last row sharing that transaction id.
    expect(priceStep?.previewRows[0]?.event).toBe(rows[0].event);
    expect(basisStep?.previewRows[0]?.event).toBe(rows[1].event);
    expect(priceStep?.previewRows[0]?.event).not.toBe(
      basisStep?.previewRows[0]?.event,
    );
  });
});

describe("quarantine reason filters", () => {
  it("keeps overlap filters when a reason spans categories", () => {
    expect(quarantineReasonFilterIds("swap_price_basis_review")).toEqual([
      "missing-prices",
      "basis-or-pairs",
    ]);
  });

  it("routes coarse pricing review into the pricing band", () => {
    expect(quarantineReasonFilterIds("pricing_review_required")).toEqual([
      "missing-prices",
    ]);
  });
});
