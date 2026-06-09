import { describe, expect, it } from "vitest";

import {
  attachmentRecordToItem,
  bucketTransactionDate,
  dashboardRecordsFromTxs,
  flowChartSelectionLabel,
  isAttachmentListQueryKeyForTransaction,
  matchesFlowChartSelection,
  removeAttachmentRecord,
  replaceAttachmentRecord,
  toDashboardTransaction,
  upsertAttachmentRecords,
  type AttachmentRecord,
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
  it("does not substitute demo rows for an empty live transaction list", () => {
    expect(dashboardRecordsFromTxs([])).toEqual([]);
  });

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

describe("transaction attachment URL labels", () => {
  it("uses daemon display labels when a saved URL row has no stored label", () => {
    const url = "https://docs.google.com/spreadsheets/d/abc123/edit?usp=sharing";
    const record: AttachmentRecord = {
      id: "att-1",
      attachment_type: "url",
      label: null,
      display_label: "Google Sheet",
      url,
      media_type: "text/uri-list",
      size_bytes: null,
      exists: null,
    };

    const item = attachmentRecordToItem(record);

    expect(item.label).toBe("Google Sheet");
    expect(item.detail).toBe(url);
  });

  it("suppresses duplicate detail for daemon-derived host path labels", () => {
    const record: AttachmentRecord = {
      id: "att-2",
      attachment_type: "url",
      label: null,
      display_label: "btcpay.example.com - abc123",
      url: "https://btcpay.example.com/invoices/abc123",
      media_type: "text/uri-list",
      size_bytes: null,
      exists: null,
    };

    const item = attachmentRecordToItem(record);

    expect(item.label).toBe("btcpay.example.com - abc123");
    expect(item.detail).toBeUndefined();
  });
});

describe("transaction attachment cache updates", () => {
  it("matches attachment list query keys by transaction", () => {
    const targetKey = [
      "daemon",
      "mock",
      0,
      "ui.attachments.list",
      { transaction: "tx-target" },
    ];
    const sourceKey = [
      "daemon",
      "mock",
      0,
      "ui.attachments.list",
      { transaction: "tx-source" },
    ];

    expect(
      isAttachmentListQueryKeyForTransaction(targetKey, "tx-target"),
    ).toBe(true);
    expect(
      isAttachmentListQueryKeyForTransaction(sourceKey, "tx-target"),
    ).toBe(false);
    expect(
      isAttachmentListQueryKeyForTransaction(
        [
          "daemon",
          "mock",
          0,
          "ui.transactions.history",
          { transaction: "tx-target" },
        ],
        "tx-target",
      ),
    ).toBe(false);
  });

  it("replaces the renamed attachment without changing the others", () => {
    const current: AttachmentRecord[] = [
      {
        id: "file-1",
        attachment_type: "file",
        display_label: "invoice.pdf",
      },
      {
        id: "url-1",
        attachment_type: "url",
        display_label: "Old link",
      },
    ];
    const updated: AttachmentRecord = {
      ...current[1],
      label: "New link",
      display_label: "New link",
    };

    expect(replaceAttachmentRecord(current, updated)).toEqual([
      current[0],
      updated,
    ]);
  });

  it("prepends newly added attachments and replaces existing records", () => {
    const current: AttachmentRecord[] = [
      {
        id: "file-1",
        attachment_type: "file",
        display_label: "invoice.pdf",
      },
      {
        id: "url-1",
        attachment_type: "url",
        display_label: "Old link",
      },
    ];
    const renamed: AttachmentRecord = {
      ...current[1],
      display_label: "Renamed link",
    };
    const created: AttachmentRecord = {
      id: "url-2",
      attachment_type: "url",
      display_label: "New link",
    };

    expect(upsertAttachmentRecords(current, [renamed, created])).toEqual([
      created,
      current[0],
      renamed,
    ]);
  });

  it("removes deleted attachment records", () => {
    const current: AttachmentRecord[] = [
      {
        id: "file-1",
        attachment_type: "file",
        display_label: "invoice.pdf",
      },
      {
        id: "url-1",
        attachment_type: "url",
        display_label: "Old link",
      },
    ];

    expect(removeAttachmentRecord(current, "url-1")).toEqual([current[0]]);
  });
});
