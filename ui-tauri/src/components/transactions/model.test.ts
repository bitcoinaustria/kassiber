import { describe, expect, it } from "vitest";

import {
  draftForTransaction,
  metadataUpdateArgs,
  formatCounterDisplayMoney,
  formatDisplayMoney,
  formatSignedDisplayMoney,
  pricingCacheSummary,
  pricingChoiceValue,
  pricingPriceMoment,
  pricingSourceLabel,
  pricingSourcePhrase,
  shouldShowSourceExternalId,
  type Transaction,
} from "./model";

function txWithTags(tags: string[]): Transaction {
  return {
    id: "tx-1",
    txnId: "source-1",
    amount: 100,
    amountBtc: 0.01,
    counterparty: "Counterparty",
    counterpartyInitials: "CP",
    direction: "Receive",
    paymentMethod: "On-chain",
    date: "2026-01-01 10:00",
    status: "completed",
    tags,
  };
}

describe("metadataUpdateArgs", () => {
  const args = (
    draft: ReturnType<typeof draftForTransaction>,
    baseline: ReturnType<typeof draftForTransaction>,
  ) =>
    metadataUpdateArgs({
      transactionId: "tx-1",
      draft,
      baseline,
      sourceTags: [],
    });

  it("sends the classification only when the user changed it", () => {
    const baseline = draftForTransaction(txWithTags([]));

    expect(args({ ...baseline, note: "just a note" }, baseline)).not.toHaveProperty(
      "kind",
    );
    expect(args({ ...baseline, kind: "income" }, baseline)).toMatchObject({
      kind: "income",
    });
    // Clearing is a real change, not an omission.
    expect(
      args({ ...baseline, kind: null }, { ...baseline, kind: "income" }),
    ).toMatchObject({ kind: null });
  });
});

describe("metadataUpdateArgs review status", () => {
  // Quarantined, so the review queue counts it under review, while the
  // user marked it completed.
  const held: Transaction = {
    ...txWithTags([]),
    reviewStatus: "completed",
    queueStatus: "review",
    quarantineReason: "missing_price",
  };
  const send = (
    draft: ReturnType<typeof draftForTransaction>,
    baseline: ReturnType<typeof draftForTransaction>,
  ) =>
    metadataUpdateArgs({ transactionId: "tx-1", draft, baseline, sourceTags: [] });

  it("edits the authored status, not the status the queue shows", () => {
    expect(draftForTransaction(held).reviewStatus).toBe("completed");
  });

  it("leaves the review status alone when only taxability changes", () => {
    const baseline = draftForTransaction(held);
    const args = send({ ...baseline, taxable: !baseline.taxable }, baseline);
    expect(args).toMatchObject({ taxable: !baseline.taxable });
    expect(args).not.toHaveProperty("review_status");
  });

  it("sends only the review status when only it changes", () => {
    const baseline = draftForTransaction(held);
    const args = send({ ...baseline, reviewStatus: "pending" }, baseline);
    expect(args).toMatchObject({ review_status: "pending" });
    expect(args).not.toHaveProperty("taxable");
    expect(args).not.toHaveProperty("at_regime");
    expect(args).not.toHaveProperty("at_category");
  });
});

describe("draftForTransaction kind", () => {
  it("never invents a tax kind for an unclassified row", () => {
    // A "Receive" row is an acquisition to the engine until the user says
    // otherwise. Defaulting here would declare income on the next save of an
    // unrelated field.
    expect(draftForTransaction(txWithTags([])).kind).toBeNull();
    expect(
      draftForTransaction({ ...txWithTags([]), tag: "Income", tags: [] }).kind,
    ).toBeNull();
  });

  it("round-trips the override so saving does not clear it", () => {
    const draft = draftForTransaction({
      ...txWithTags([]),
      kindOverride: "mining",
    });

    expect(draft.kind).toBe("mining");
  });

  it("does not adopt the importer's provenance kind as a classification", () => {
    // `kind` is what the source said. Seeding the draft from it would re-save
    // provenance as a user classification — and `lnd_invoice` is not even a
    // kind the daemon accepts.
    const draft = draftForTransaction({
      ...txWithTags([]),
      kind: "lnd_invoice",
    });

    expect(draft.kind).toBeNull();
  });
});

describe("draftForTransaction", () => {
  it("preserves additional label-like tags outside the selected classification", () => {
    const draft = draftForTransaction(txWithTags(["Income", "Review", "Fee"]));

    expect(draft.label).toBe("Income");
    expect(draft.tags).toEqual(["Review", "Fee"]);
  });

  it("does not infer a durable classification from display fallback tags", () => {
    const draft = draftForTransaction({
      ...txWithTags([]),
      tag: "Income",
      tags: [],
    });

    expect(draft.label).toBe("Unlabeled");
    expect(draft.tags).toEqual([]);
  });

  it("keeps reviewed swap movements tax-neutral by default", () => {
    const draft = draftForTransaction({
      ...txWithTags(["Swap"]),
      direction: "Transfer",
      flow: "swap",
    });

    expect(draft.atCategory).toBe("neu_swap");
    expect(draft.taxable).toBe(false);
  });

  it("hydrates persisted pricing provenance into the edit draft", () => {
    const draft = draftForTransaction({
      ...txWithTags([]),
      amount: 6500,
      rate: 65000,
      fiatCurrency: "EUR",
      pricingSourceKind: "manual_override",
      pricingQuality: "exact",
      pricingExternalRef: "Invoice 42",
    });

    expect(draft.pricingSourceKind).toBe("manual_override");
    expect(draft.pricingQuality).toBe("exact");
    expect(draft.manualCurrency).toBe("EUR");
    expect(draft.manualPrice).toBe("65000");
    expect(draft.manualValue).toBe("6500");
    expect(draft.manualSource).toBe("Invoice 42");
  });

  it("hydrates persisted review and tax handling into the edit draft", () => {
    const draft = draftForTransaction({
      ...txWithTags([]),
      reviewStatus: "review",
      taxable: false,
      atRegime: "outside",
      atCategory: "none",
    });

    expect(draft.reviewStatus).toBe("review");
    expect(draft.taxable).toBe(false);
    expect(draft.atRegime).toBe("outside");
    expect(draft.atCategory).toBe("none");
  });
});

describe("money formatting", () => {
  it("renders missing fiat values as unpriced instead of zero", () => {
    expect(formatDisplayMoney(null, 0.01, "eur")).toBe("Unpriced");
    expect(formatSignedDisplayMoney(null, 0.01, "eur")).toBe("Unpriced");
    expect(formatCounterDisplayMoney(null, 0.01, "btc")).toBe("Unpriced");
  });
});

describe("pricing provenance", () => {
  it("summarizes provider cache provenance without flattening the quality tier", () => {
    expect(
      pricingCacheSummary({
        ...txWithTags([]),
        pricingSourceKind: "fmv_provider",
        pricingQuality: "coarse_fallback",
        pricingProvider: "kraken-csv",
        pricingPair: "BTC-EUR",
        pricingGranularity: "daily",
      }),
    ).toBe("Kraken CSV · BTC-EUR · daily");
  });

  it("shifts the Kraken OHLCVT close back to the trading day", () => {
    expect(
      pricingPriceMoment({
        ...txWithTags([]),
        pricingGranularity: "daily",
        pricingProvider: "kraken-csv",
        pricingMethod: "ohlcvt_csv",
        pricingTimestamp: "2024-05-02T00:00:00Z",
      }),
    ).toEqual({ label: "transactions:pricePoint.tradingDay", value: "2024-05-01" });
  });

  it("does not shift non-Kraken daily rows that are already day-stamped", () => {
    expect(
      pricingPriceMoment({
        ...txWithTags([]),
        pricingGranularity: "daily",
        pricingProvider: "coingecko",
        pricingMethod: "market_chart",
        pricingTimestamp: "2024-05-01T00:00:00Z",
      }),
    ).toEqual({ label: "transactions:pricePoint.tradingDay", value: "2024-05-01" });
  });

  it("reports the precise timestamp for minute candles", () => {
    expect(
      pricingPriceMoment({
        ...txWithTags([]),
        pricingGranularity: "minute",
        pricingTimestamp: "2024-05-01T00:02:00Z",
      }),
    ).toEqual({ label: "transactions:pricePoint.timestamp", value: "2024-05-01 00:02" });
  });
});

describe("source identifiers", () => {
  it("hides source external id when it duplicates the displayed transaction id", () => {
    expect(
      shouldShowSourceExternalId({
        txnId: "1641779af4",
        explorerId: "1641779af4",
      }),
    ).toBe(false);
  });

  it("shows source external id when it differs from the explorer transaction id", () => {
    expect(
      shouldShowSourceExternalId({
        txnId: "btcpay-payment-42",
        explorerId: "1641779af4",
      }),
    ).toBe(true);
  });
});

describe("pricing choices", () => {
  it("groups every source kind under one of the four choices, as the core does", () => {
    expect(pricingChoiceValue("exchange_execution", "exact")).toBe("generic_import");
    expect(pricingChoiceValue("wallet_export", "exact")).toBe("generic_import");
    expect(pricingChoiceValue("btcpay_invoice", "exact")).toBe("generic_import");
    expect(pricingChoiceValue("generic_import", "provider_sample")).toBe("generic_import");
    expect(pricingChoiceValue("fmv_provider", "coarse_fallback")).toBe("fmv_provider");
    expect(pricingChoiceValue("manual_rate_cache", "exact")).toBe("fmv_provider");
    expect(pricingChoiceValue("manual_override", "exact")).toBe("manual_override");
    expect(pricingChoiceValue("exchange_execution", "missing")).toBe("missing");
    expect(pricingChoiceValue(null, "exact")).toBe("missing");
  });

  it("names a source kind outside the choices instead of showing its code", () => {
    expect(pricingSourceLabel("exchange_execution", "exact")).toBe(
      "transactions:pricingSourceKind.exchange_execution.label",
    );
    expect(pricingSourcePhrase("exchange_execution", "exact")).toBe(
      "transactions:pricingSourceKind.exchange_execution.phrase",
    );
    expect(pricingSourcePhrase(null, "missing")).toBeNull();
  });
});
