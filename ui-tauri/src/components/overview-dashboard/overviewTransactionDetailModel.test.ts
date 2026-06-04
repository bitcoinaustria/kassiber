import { describe, expect, it } from "vitest";

import type { OverviewSnapshot, Tx } from "@/mocks/seed";
import { toDashboardTransaction } from "./model";
import { overviewDetailTransactions } from "./overviewTransactionDetailModel";

function tx(overrides: Partial<Tx> & Pick<Tx, "id">): Tx {
  const { id, ...rest } = overrides;
  return {
    id,
    date: "2026-01-01 10:00",
    type: "Income",
    account: "Cold wallet",
    counter: "Customer",
    amountSat: 100_000,
    eur: 100,
    rate: 100_000,
    tag: "Income",
    conf: 1,
    ...rest,
  };
}

function snapshot(overrides: Partial<OverviewSnapshot>): OverviewSnapshot {
  return {
    priceEur: 100_000,
    priceUsd: 110_000,
    connections: [],
    txs: [],
    balanceSeries: [],
    fiat: {
      eurBalance: 0,
      eurCostBasis: 0,
      eurUnrealized: 0,
      eurRealizedYTD: 0,
    },
    ...overrides,
  };
}

describe("overviewDetailTransactions", () => {
  it("shows fee-only consolidations as transfer rows with the fee amount", () => {
    const record = toDashboardTransaction(
      tx({
        id: "tx12",
        type: "Consolidation",
        amountSat: 0,
        feeSat: 42_180,
        eur: -30.13,
        tag: "Consolidation fee",
      }),
      0,
    );

    expect(record.flow).toBe("transfer");
    expect(record.amountBtc).toBe(-0.0004218);
    expect(record.amount).toBe(-30.13);
  });

  it("includes activity-only rows so chart markers can open details", () => {
    const records = overviewDetailTransactions(
      snapshot({
        txs: [tx({ id: "recent-tx" })],
        activityTxs: [tx({ id: "older-activity-tx", type: "Swap", tag: "Swap" })],
      }),
    );

    expect(records.map((record) => record.id)).toEqual([
      "recent-tx",
      "older-activity-tx",
    ]);
  });

  it("keeps the richer recent row when a transaction also appears in activity", () => {
    const records = overviewDetailTransactions(
      snapshot({
        txs: [
          tx({
            id: "tx-1",
            tag: "Reviewed, Income",
            tags: ["Reviewed", "Income"],
          }),
        ],
        activityTxs: [tx({ id: "tx-1", tag: "Income", tags: ["Income"] })],
      }),
    );

    expect(records).toHaveLength(1);
    expect(records[0]?.tags).toEqual(["Reviewed", "Income"]);
  });
});
