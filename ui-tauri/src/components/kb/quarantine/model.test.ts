import { describe, expect, it } from "vitest";

import i18n from "@/i18n";

import {
  quarantineItemToRow,
  quarantineMetrics,
  quarantineReasonFilterIds,
} from "./model";
import type { QuarantineItem } from "./types";

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
      event: "Missing Spot Price",
      source: "Journal quarantine · outbound",
      amount: "-1,000,000 sats BTC",
      basis: "Missing fiat price",
      impact: "Held for review",
      status: "Needs review",
      priority: "Medium",
      owner: "AT profile",
      evidenceHint: "Add a fiat price or rates coverage",
      nextAction: "Set price, then process journals",
      metricFilterIds: ["missing-prices"],
      transactionAction: {
        transactionId: "transaction-with-a-long-id",
        label: "Open pricing",
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
    expect(row.impact).toBe("Blocks reports");
    expect(row.owner).toBe("Active book");
    expect(row.amount).toBe("250,000 sats BTC");
    expect(row.metricFilterIds).toEqual(["basis-or-pairs"]);
    expect(row.transactionAction).toMatchObject({
      transactionId: "transaction-with-a-long-id",
      label: "Open tax review",
      tab: "tax",
    });
  });

  it("treats coarse pricing review as a non-blocking pricing task", () => {
    const row = quarantineItemToRow(
      {
        ...baseItem,
        reason: "pricing_review_required",
      },
      "AT profile",
      t,
    );

    expect(row).toMatchObject({
      event: "Pricing Review Required",
      basis: "Coarse pricing — verify",
      status: "Needs review",
      priority: "Medium",
      evidenceHint: "Priced from daily rates — fetch precise prices or confirm",
      nextAction: "Fetch precise prices or confirm, then process journals",
      metricFilterIds: ["missing-prices"],
      transactionAction: {
        label: "Open pricing",
        tab: "pricing",
      },
    });
  });

  it("describes implausible transfer fees as split-transfer review", () => {
    const row = quarantineItemToRow(
      {
        ...baseItem,
        reason: "transfer_fee_implausible",
      },
      "AT profile",
      t,
    );

    expect(row).toMatchObject({
      basis: "Split transfer / swap review",
      evidenceHint: "Review the self-transfer and residual swap or payout leg",
      nextAction: "Review split transfer/swap, then process journals",
      transactionAction: {
        label: "Open transaction",
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
      event: "Ownership Transfer Destination Ambiguous",
      basis: "Self-transfer between your wallets",
      status: "Needs review",
      priority: "Medium",
      evidenceHint:
        "Proven from the on-chain transaction — an output paid another of your wallets, but the matching receipt couldn't be resolved automatically",
      nextAction:
        "Pair it manually, or sync the destination wallet so its receipt is recorded, then process journals",
      metricFilterIds: ["basis-or-pairs"],
      transactionAction: {
        label: "Open pairing",
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
