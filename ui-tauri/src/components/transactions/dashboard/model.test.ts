import { afterEach, describe, expect, it, vi } from "vitest";

import i18n from "@/i18n";

import {
  attachmentRecordToItem,
  availablePeriodKeysForRecords,
  buildCandidateFlowOverrides,
  buildFlowChartRows,
  buildSwapCandidates,
  buildTransactionListFilterArgs,
  buildTransferCandidates,
  bucketTransactionDate,
  candidateReferenceReviewType,
  dashboardRecordsFromTxs,
  flowChartSelectionLabel,
  flowChartSelectionServerFlow,
  flowChartSelectionDateWindow,
  initialPeriodFromUrl,
  isAttachmentListQueryKeyForTransaction,
  matchesFlowChartSelection,
  readTransactionDetailParams,
  readTransactionScopeParams,
  serializeTransactionFilterParams,
  removeAttachmentRecord,
  replaceAttachmentRecord,
  resolveAutoPeriodForRecords,
  toDashboardTransaction,
  transactionListPeriodFilter,
  transactionPeriodDateWindow,
  transactionFlowWithCandidateOverrides,
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
        transactionIds: [],
        period: null,
        flowChartSelection: null,
        breakdownSelection: {
          dimension: "wallet",
          key: "Satoshi-Liquid -> Satoshi-Onchain-Multi",
          match: "leg",
        },
        table: {
          status: "all",
          flow: "all",
          paymentMethod: "all",
          fee: "all",
          sort: null,
        },
      });
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("preserves clustered transaction id filters in scope parameters", () => {
    vi.stubGlobal("window", {
      location: {
        search: "?txids=tx-one,tx-two&quick=review_queue",
      },
    });

    try {
      expect(readTransactionScopeParams()).toEqual({
        wallet: null,
        quick: "review_queue",
        transactionIds: ["tx-one", "tx-two"],
        period: null,
        flowChartSelection: null,
        breakdownSelection: null,
        table: {
          status: "all",
          flow: "all",
          paymentMethod: "all",
          fee: "all",
          sort: null,
        },
      });
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("lets an exact chart cluster override the broad transaction period", () => {
    expect(
      transactionListPeriodFilter("30days", ["older-chart-event"]),
    ).toBeNull();
    expect(transactionListPeriodFilter("1year", [])).toBe("1year");
    expect(transactionListPeriodFilter("all", [])).toBeNull();
  });

  it("builds exact cluster requests without leaking the broad period", () => {
    expect(
      buildTransactionListFilterArgs({
        period: "30days",
        transactionIds: ["older-chart-event"],
        flowChartSelection: null,
        quickFilter: null,
        breakdownSelection: null,
        tableFilterState: {
          status: "all",
          flow: "all",
          paymentMethod: "all",
          fee: "all",
          sort: null,
        },
      }),
    ).toEqual({ txids: ["older-chart-event"] });
  });

  it("uses one exact local-calendar window for charts and list requests", () => {
    const now = new Date(2024, 2, 31, 12, 30);
    const window = transactionPeriodDateWindow("3months", now);
    expect(window).toEqual({
      since: new Date(2023, 11, 31, 0, 0, 0, 0).toISOString(),
      until: new Date(2024, 2, 31, 23, 59, 59, 999).toISOString(),
    });
    expect(
      buildTransactionListFilterArgs({
        period: "3months",
        transactionIds: [],
        flowChartSelection: null,
        quickFilter: null,
        breakdownSelection: null,
        tableFilterState: {
          status: "all",
          flow: "all",
          paymentMethod: "all",
          fee: "all",
          sort: null,
        },
        now,
      }),
    ).toEqual(window);
  });

  it("clamps partial edge buckets to the chart period's exact window", () => {
    const now = new Date(2026, 6, 18, 12, 30);
    const tableFilterState = {
      status: "all",
      flow: "all",
      paymentMethod: "all",
      fee: "all" as const,
      sort: null,
    };
    expect(
      buildTransactionListFilterArgs({
        period: "1year",
        transactionIds: [],
        flowChartSelection: {
          id: "1year:2025-07:incoming:external",
          period: "1year",
          bucketKey: "2025-07",
          bucketLabel: "Jul 25",
          segment: "incoming",
          mode: "external",
        },
        quickFilter: null,
        breakdownSelection: null,
        tableFilterState,
        now,
      }),
    ).toMatchObject({
      since: new Date(2025, 6, 18, 0, 0, 0, 0).toISOString(),
      until: new Date(2025, 7, 1, 0, 0, 0, -1).toISOString(),
    });
    expect(
      buildTransactionListFilterArgs({
        period: "30days",
        transactionIds: [],
        flowChartSelection: {
          id: "1year:2026-07:outgoing:external",
          period: "1year",
          bucketKey: "2026-07",
          bucketLabel: "Jul 26",
          segment: "outgoing",
          mode: "external",
        },
        quickFilter: null,
        breakdownSelection: null,
        tableFilterState,
        now,
      }),
    ).toMatchObject({
      since: new Date(2026, 6, 1).toISOString(),
      until: new Date(2026, 6, 18, 23, 59, 59, 999).toISOString(),
    });
  });

  it("requests the backend amount order for the displayed currency", () => {
    const base = {
      period: "all" as const,
      transactionIds: [],
      flowChartSelection: null,
      quickFilter: null,
      breakdownSelection: null,
      tableFilterState: {
        status: "all",
        flow: "all",
        paymentMethod: "all",
        fee: "all" as const,
        sort: { key: "amount" as const, direction: "asc" as const },
      },
    };
    expect(buildTransactionListFilterArgs({ ...base, currency: "btc" })).toMatchObject({
      sort: "amount",
      order: "asc",
    });
    expect(buildTransactionListFilterArgs({ ...base, currency: "eur" })).toMatchObject({
      sort: "fiat-value",
      order: "asc",
    });
  });

  it("round-trips the controlled filter state through URL parameters", () => {
    const state = {
      period: "1year" as const,
      flowChartSelection: null,
      quickFilter: "review_queue" as const,
      breakdownSelection: {
        dimension: "wallet" as const,
        key: "Cold Storage",
        match: "leg" as const,
      },
      transactionIds: ["tx-one", "tx-two"],
      table: {
        status: "review",
        flow: "outgoing",
        paymentMethod: "Lightning",
        fee: "with-fees" as const,
        sort: { key: "amount" as const, direction: "desc" as const },
      },
    };
    const search = serializeTransactionFilterParams("?tx=detail", state);
    vi.stubGlobal("window", { location: { search: `?${search}` } });
    expect(readTransactionScopeParams()).toMatchObject({
      wallet: "Cold Storage",
      quick: "review_queue",
      transactionIds: ["tx-one", "tx-two"],
      period: "1year",
      table: state.table,
    });
    expect(new URLSearchParams(search).get("tx")).toBe("detail");
  });

  it("preserves a chart filter's own period when the visible period changes", () => {
    const search = serializeTransactionFilterParams("", {
      period: "30days",
      flowChartSelection: {
        id: "5years:2024-Q1:swaps:all",
        period: "5years",
        bucketKey: "2024-Q1",
        bucketLabel: "Q1 2024",
        segment: "swaps",
        mode: "all",
      },
      quickFilter: null,
      breakdownSelection: null,
      transactionIds: [],
      table: {
        status: "all",
        flow: "all",
        paymentMethod: "all",
        fee: "all",
        sort: null,
      },
    });
    const parsed = readTransactionScopeParams(search);
    expect(parsed.period).toBe("30days");
    expect(parsed.flowChartSelection).toMatchObject({
      period: "5years",
      bucketKey: "2024-Q1",
      segment: "swaps",
    });
  });

  it("keeps transaction request filter precedence explicit", () => {
    expect(
      buildTransactionListFilterArgs({
        period: "1year",
        transactionIds: [],
        flowChartSelection: {
          id: "bucket-1",
          period: "1year",
          bucketKey: "2026-04",
          bucketLabel: "Apr 26",
          segment: "incoming",
          mode: "external",
        },
        quickFilter: "missing_price",
        breakdownSelection: { dimension: "network", key: "Lightning" },
        tableFilterState: {
          status: "review",
          flow: "outgoing",
          paymentMethod: "On-chain",
          fee: "with-fees",
          sort: { key: "amount", direction: "desc" },
        },
      }),
    ).toEqual({
      since: new Date(2026, 3, 1).toISOString(),
      until: new Date(2026, 4, 1, 0, 0, 0, -1).toISOString(),
      flow: "outgoing",
      quick: "missing_price",
      payment_method: "On-chain",
      status: "review",
      withFees: true,
      sort: "amount",
      order: "desc",
    });
  });

  it("maps chart buckets to daemon date windows", () => {
    expect(
      flowChartSelectionDateWindow({
        id: "bucket-1",
        period: "1year",
        bucketKey: "2026-04",
        bucketLabel: "Apr 26",
        segment: "incoming",
        mode: "external",
      }),
    ).toEqual({
      since: new Date(2026, 3, 1).toISOString(),
      until: new Date(2026, 4, 1, 0, 0, 0, -1).toISOString(),
    });

    expect(
      flowChartSelectionDateWindow({
        id: "bucket-2",
        period: "5years",
        bucketKey: "2026-Q2",
        bucketLabel: "Q2 26",
        segment: "swaps",
        mode: "all",
      }),
    ).toEqual({
      since: new Date(2026, 3, 1).toISOString(),
      until: new Date(2026, 6, 1, 0, 0, 0, -1).toISOString(),
    });
  });

  it("hides long-range period tabs for young transaction histories", () => {
    expect(
      availablePeriodKeysForRecords([
        transaction({ id: "newer", date: "2100-01-01T12:00:00Z" }),
        transaction({ id: "older", date: "2099-08-01T12:00:00Z" }),
      ]),
    ).toEqual(["30days", "3months", "6months", "ytd", "1year", "all"]);
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
      "6months",
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
      "6months",
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

  it("parses auto period URLs and falls back to a stored period", () => {
    vi.stubGlobal("window", { location: { search: "" } });
    expect(initialPeriodFromUrl("5years")).toBe("5years");

    vi.stubGlobal("window", { location: { search: "?period=auto" } });
    expect(initialPeriodFromUrl("5years")).toBe("auto");

    vi.stubGlobal("window", { location: { search: "?period=6m" } });
    expect(initialPeriodFromUrl("5years")).toBe("6months");
  });

  it("resolves auto to the smallest useful transaction period", () => {
    expect(
      resolveAutoPeriodForRecords(
        [
          transaction({ id: "recent-1", date: "2026-06-28T12:00:00Z" }),
          transaction({ id: "recent-2", date: "2026-06-20T12:00:00Z" }),
          transaction({ id: "recent-3", date: "2026-06-10T12:00:00Z" }),
        ],
        "auto",
      ),
    ).toBe("ytd");

    expect(
      resolveAutoPeriodForRecords(
        [
          transaction({ id: "old-1", date: "2026-01-20T12:00:00Z" }),
          transaction({ id: "old-2", date: "2026-01-10T12:00:00Z" }),
          transaction({ id: "old-3", date: "2025-12-15T12:00:00Z" }),
        ],
        "auto",
      ),
    ).toBe("1year");
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

  it("preserves daemon asset and chain metadata on transaction rows", () => {
    const transaction = toDashboardTransaction(
      rawTx({
        asset: "LBTC",
        chain: "liquid",
        network: "liquidv1",
        account: "Wallet export",
      }),
      0,
    );

    expect(transaction.asset).toBe("LBTC");
    expect(transaction.chain).toBe("liquid");
    expect(transaction.network).toBe("liquidv1");
    expect(transaction.paymentMethod).toBe("Liquid");
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

  it("pushes every chart segment into the matching daemon flow filter", () => {
    const baseSelection: FlowChartSelection = {
      id: "all:all:transfers:all",
      period: "all",
      bucketKey: null,
      bucketLabel: "All",
      segment: "transfers",
      mode: "all",
    };

    expect(flowChartSelectionServerFlow(baseSelection)).toBe("transfer");
    expect(
      flowChartSelectionServerFlow({
        ...baseSelection,
        id: "all:all:swaps:all",
        segment: "swaps",
      }),
    ).toBe("swap");
    expect(
      flowChartSelectionServerFlow({
        ...baseSelection,
        id: "all:all:incoming:all",
        segment: "incoming",
      }),
    ).toBe("incoming");
    expect(
      flowChartSelectionServerFlow({
        ...baseSelection,
        id: "all:all:outgoing:all",
        segment: "outgoing",
      }),
    ).toBe("outgoing");
  });

  it("includes candidate legs when server-filtering a candidate-backed bar", () => {
    expect(
      buildTransactionListFilterArgs({
        period: "all",
        transactionIds: [],
        flowChartSelection: {
          id: "all:all:swaps:all",
          period: "all",
          bucketKey: null,
          bucketLabel: "All",
          segment: "swaps",
          mode: "all",
        },
        quickFilter: null,
        breakdownSelection: null,
        tableFilterState: {
          status: "all",
          flow: "all",
          paymentMethod: "all",
          fee: "all",
          sort: null,
        },
        pairingCandidateRefs: [
          {
            in_id: "swap-in",
            out_id: "swap-out",
            in_asset: "LBTC",
            out_asset: "BTC",
          },
        ],
      }),
    ).toEqual({
      flow: "swap",
      candidate_txids: ["swap-in", "swap-out"],
    });
  });

  it("treats transfer candidate legs as transfers in table chart filters", () => {
    const selection: FlowChartSelection = {
      id: "all:all:transfers:all",
      period: "all",
      bucketKey: null,
      bucketLabel: "All",
      segment: "transfers",
      mode: "all",
    };
    const outgoingLeg = transaction({
      id: "tx-out",
      direction: "Send",
      flow: "outgoing",
    });
    const incomingLeg = transaction({
      id: "tx-in",
      direction: "Receive",
      flow: "incoming",
    });
    const transferCandidateIds = new Set(["tx-out", "tx-in"]);
    const displayFlow = (txn: Transaction) =>
      transactionFlowWithCandidateOverrides(txn, new Set(), transferCandidateIds);

    expect(matchesFlowChartSelection(outgoingLeg, selection, displayFlow)).toBe(true);
    expect(matchesFlowChartSelection(incomingLeg, selection, displayFlow)).toBe(true);
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

    const candidateFlows = buildCandidateFlowOverrides([out, input], [candidate], {
      transferFlow: "layer-transition",
    });
    expect(candidateFlows.transferCandidateIds).toEqual(
      new Set(["tx-out", "tx-in"]),
    );

    const rows = buildFlowChartRows(
      [out, input],
      "1year",
      "btc",
      candidateFlows.flowById,
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
