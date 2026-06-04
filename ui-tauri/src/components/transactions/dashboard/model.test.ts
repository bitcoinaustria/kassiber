import { describe, expect, it } from "vitest";

import {
  bucketTransactionDate,
  flowChartSelectionLabel,
  matchesFlowChartSelection,
  toDashboardTransaction,
  type FlowChartSelection,
} from "./model";
import type { Tx } from "@/mocks/seed";
import type { Transaction, TransactionFlow } from "@/components/transactions";

function transaction(
  overrides: Partial<Transaction> = {},
): Transaction {
  return {
    id: "tx-1",
    txnId: "txid-1",
    amount: 10,
    amountBtc: 0.01,
    counterparty: "Alice",
    counterpartyInitials: "AL",
    direction: "Receive",
    paymentMethod: "On-chain",
    date: "2026-04-15T12:00:00Z",
    status: "completed",
    ...overrides,
  };
}

describe("transaction dashboard chart selection", () => {
  it("classifies consolidation rows as transfers even when only the fee is negative", () => {
    const tx: Tx = {
      id: "tx12",
      date: "2026-04-03 09:22",
      type: "Consolidation",
      account: "Cold Storage",
      counter: "12 UTXOs -> 1",
      amountSat: 0,
      feeSat: 42_180,
      eur: -30.13,
      rate: 71432.0,
      tag: "Consolidation fee",
      conf: 210,
    };

    const transaction = toDashboardTransaction(tx, 0);

    expect(transaction.flow).toBe("transfer");
    expect(transaction.direction).toBe("Transfer");
    expect(transaction.amountBtc).toBe(0.0004218);
    expect(transaction.amount).toBe(30.13);
    expect(transaction.feeBtc).toBe(0.0004218);
  });

  it("labels and matches a whole bucket selection across flows", () => {
    const bucket = bucketTransactionDate(
      new Date("2026-04-15T12:00:00Z"),
      "1year",
    );
    const selection: FlowChartSelection = {
      id: `1year:${bucket.key}:all:all`,
      period: "1year",
      bucketKey: bucket.key,
      bucketLabel: bucket.label,
      segment: null,
      mode: "all",
    };

    expect(flowChartSelectionLabel(selection)).toBe(
      `${bucket.label} · All flows · All`,
    );
    expect(
      matchesFlowChartSelection(
        transaction({ flow: "incoming" }),
        selection,
        (txn) => txn.flow as TransactionFlow,
      ),
    ).toBe(true);
    expect(
      matchesFlowChartSelection(
        transaction({
          id: "tx-2",
          txnId: "txid-2",
          date: "2026-05-15T12:00:00Z",
          flow: "incoming",
        }),
        selection,
        (txn) => txn.flow as TransactionFlow,
      ),
    ).toBe(false);
  });

  it("limits whole bucket selections to visible flows in external mode", () => {
    const bucket = bucketTransactionDate(
      new Date("2026-04-15T12:00:00Z"),
      "1year",
    );
    const selection: FlowChartSelection = {
      id: `1year:${bucket.key}:all:external`,
      period: "1year",
      bucketKey: bucket.key,
      bucketLabel: bucket.label,
      segment: null,
      mode: "external",
    };

    expect(
      matchesFlowChartSelection(
        transaction({ flow: "incoming" }),
        selection,
        (txn) => txn.flow as TransactionFlow,
      ),
    ).toBe(true);
    expect(
      matchesFlowChartSelection(
        transaction({ flow: "transfer" }),
        selection,
        (txn) => txn.flow as TransactionFlow,
      ),
    ).toBe(false);
    expect(
      matchesFlowChartSelection(
        transaction({ flow: "swap" }),
        selection,
        (txn) => txn.flow as TransactionFlow,
      ),
    ).toBe(false);
  });
});
