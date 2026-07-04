import { afterEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

import {
  attachmentRecordToItem,
  availablePeriodKeysForRecords,
  buildFlowChartRows,
  buildSwapCandidates,
  buildTransferCandidates,
  bucketTransactionDate,
  candidateReferenceReviewType,
  dashboardRecordsFromTxs,
  flowChartSelectionLabel,
  isAttachmentListQueryKeyForTransaction,
  matchesFlowChartSelection,
  readTransactionDetailParams,
  readTransactionScopeParams,
  removeAttachmentRecord,
  replaceAttachmentRecord,
  toDashboardTransaction,
  upsertAttachmentRecords,
  type AttachmentRecord,
  type FlowChartSelection,
} from "./model";
import type { Tx } from "@/mocks/seed";
import type { Transaction, TransactionFlow } from "@/components/transactions";

const t = i18n.getFixedT("en", "transactions");

afterEach(() => {
  vi.unstubAllGlobals();
});

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

function rawTx(overrides: Partial<Tx> = {}): Tx {
  return {
    id: "tx-1",
    date: "2026-04-15 08:00",
    type: "Income",
    account: "Cold Storage",
    counter: "Counterparty",
    amountSat: 410_000,
    eur: null,
    rate: null,
    tag: "Income",
    conf: 32,
    ...overrides,
  };
}

describe("transaction dashboard chart selection", () => {
  it("preserves quarantine row ids in transaction detail deep links", () => {
    vi.stubGlobal("window", {
      location: {
        search: "?tx=shared-tx&tab=linked&qrow=shared-tx%3A2",
      },
    });

    try {
      expect(readTransactionDetailParams()).toEqual({
        transactionId: "shared-tx",
        tab: "linked",
        rowId: "shared-tx:2",
      });
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("preserves wallet detail deep-link scope parameters", () => {
    vi.stubGlobal("window", {
      location: {
        search:
          "?wallet=Satoshi-Liquid%20-%3E%20Satoshi-Onchain-Multi&quick=missing_price",
      },
    });

    try {
      expect(readTransactionScopeParams()).toEqual({
        wallet: "Satoshi-Liquid -> Satoshi-Onchain-Multi",
        quick: "missing_price",
      });
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("hides long-range period tabs for young transaction histories", () => {
    expect(
      availablePeriodKeysForRecords([
        transaction({ id: "newer", date: "2100-01-01T12:00:00Z" }),
        transaction({ id: "older", date: "2099-08-01T12:00:00Z" }),
      ]),
    ).toEqual(["30days", "3months", "ytd", "1year", "all"]);
  });

  it("reveals longer period tabs as transaction history gets older", () => {
    expect(
      availablePeriodKeysForRecords([
        transaction({ id: "newer", date: "2100-01-01T12:00:00Z" }),
        transaction({ id: "older", date: "2084-01-01T12:00:00Z" }),
      ]),
    ).toEqual([
      "30days",
      "3months",
      "ytd",
      "1year",
      "5years",
      "10years",
      "15years",
      "all",
    ]);
  });

  it("reveals long periods when the oldest bound extends a recent workbench slice", () => {
    const recentSlice = Array.from({ length: 500 }, (_, index) =>
      transaction({
        id: `recent-${index}`,
        date: `2026-04-${String((index % 28) + 1).padStart(2, "0")}T12:00:00Z`,
      }),
    );

    expect(availablePeriodKeysForRecords(recentSlice)).toEqual([
      "30days",
      "3months",
      "ytd",
      "1year",
      "all",
    ]);
    expect(
      availablePeriodKeysForRecords([
        ...recentSlice,
        transaction({ id: "oldest-bound", date: "2019-01-15T09:00:00Z" }),
      ]),
    ).toContain("5years");
  });

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

  it("normalizes backend review-status variants before detail rendering", () => {
    const tx = rawTx({
      id: "tx-missing-price",
      counter: "Missing price",
      tag: "Review",
      reviewStatus: "needs_review",
    });

    expect(toDashboardTransaction(tx, 0).reviewStatus).toBe("review");
  });

  it.each([
    ["review", "review"],
    ["needs_review", "review"],
    ["needs-review", "review"],
    ["blocked", "review"],
    ["quarantined", "review"],
    ["completed", "completed"],
    ["complete", "completed"],
    ["pending", "pending"],
    ["failed", "failed"],
    ["error", "failed"],
  ] as const)(
    "normalizes backend review status %s as %s",
    (rawStatus, expectedStatus) => {
      expect(
        toDashboardTransaction(rawTx({ reviewStatus: rawStatus }), 0).reviewStatus,
      ).toBe(expectedStatus);
    },
  );

  it("falls back to confirmation-derived status for blank or unknown review statuses", () => {
    expect(
      toDashboardTransaction(rawTx({ reviewStatus: "", conf: 0 }), 0).reviewStatus,
    ).toBe("pending");
    expect(
      toDashboardTransaction(
        rawTx({ reviewStatus: "waiting_for_oracle", conf: 12 }),
        0,
      ).reviewStatus,
    ).toBe("completed");
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

    // loose translator
    expect(
      flowChartSelectionLabel(
        selection,
        t as (key: string, opts?: Record<string, unknown>) => string,
      ),
    ).toBe(
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

  it("treats BTC to Liquid BTC candidates as carrying-value Bitcoin swaps", () => {
    const out = transaction({
      id: "tx-out",
      direction: "Send",
      flow: "outgoing",
      asset: "BTC",
      amountBtc: 0.1,
      amount: 6000,
    });
    const input = transaction({
      id: "tx-in",
      direction: "Receive",
      flow: "incoming",
      asset: "LBTC",
      amountBtc: 0.0999,
      amount: 5994,
    });
    const candidate = {
      out_id: "tx-out",
      in_id: "tx-in",
      out_asset: "BTC",
      in_asset: "LBTC",
      default_kind: "peg-in",
      candidate_type: "transfer" as const,
    };

    expect(candidateReferenceReviewType(candidate)).toBe("transfer");
    expect(buildSwapCandidates([out, input], [candidate])).toEqual([]);
    expect(buildTransferCandidates([out, input], [candidate])).toHaveLength(1);

    const rows = buildFlowChartRows(
      [out, input],
      "1year",
      "btc",
      new Map<string, TransactionFlow>([
        ["tx-out", "layer-transition"],
        ["tx-in", "layer-transition"],
      ]),
      "count",
    );
    expect(rows.find((row) => row.transfers === 2)?.swaps).toBe(0);
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

describe("transaction detail routing", () => {
  it("falls back to details for the old graph deep-link tab", () => {
    vi.stubGlobal("window", {
      location: {
        search: "?tx=tx-graph&tab=graph",
        pathname: "/transactions",
      },
      history: {
        replaceState: vi.fn(),
      },
    });

    expect(readTransactionDetailParams()).toEqual({
      transactionId: "tx-graph",
      tab: "details",
    });
  });

  it("falls back to details for the old ledger deep-link tab", () => {
    vi.stubGlobal("window", {
      location: {
        search: "?tx=tx-ledger&tab=ledger",
        pathname: "/transactions",
      },
      history: {
        replaceState: vi.fn(),
      },
    });

    expect(readTransactionDetailParams()).toEqual({
      transactionId: "tx-ledger",
      tab: "details",
    });
  });
});
