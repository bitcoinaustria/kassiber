import { describe, expect, it } from "vitest";

import {
  quarantineItemToRow,
  quarantineMetrics,
  quarantineReasonFilterIds,
} from "./model";
import type { QuarantineItem } from "./types";

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
    const row = quarantineItemToRow(baseItem, "AT profile");

    expect(row).toMatchObject({
      id: "transactio...",
      date: "2026-05-01",
      account: "Cold",
      event: "Missing Spot Price",
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
    );

    expect(row.status).toBe("Blocked");
    expect(row.priority).toBe("High");
    expect(row.owner).toBe("Active book");
    expect(row.amount).toBe("250,000 sats BTC");
    expect(row.metricFilterIds).toEqual(["basis-or-pairs"]);
  });
});

describe("quarantine metrics", () => {
  it("groups daemon reason strings into review filters", () => {
    const metrics = quarantineMetrics({
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
    });

    expect(metrics.map((metric) => [metric.label, metric.value])).toEqual([
      ["Quarantined", 7],
      ["Missing prices", 2],
      ["Basis or pairs", 4],
      ["Other review", 1],
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
});
