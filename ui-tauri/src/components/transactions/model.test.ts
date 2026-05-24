import { describe, expect, it } from "vitest";

import {
  draftForTransaction,
  formatCounterDisplayMoney,
  formatDisplayMoney,
  formatSignedDisplayMoney,
  pricingCacheSummary,
  pricingPriceMoment,
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

  it("reports the trading day for daily candles stored at their close", () => {
    expect(
      pricingPriceMoment({
        ...txWithTags([]),
        pricingGranularity: "daily",
        pricingTimestamp: "2024-05-02T00:00:00Z",
      }),
    ).toEqual({ label: "Trading day", value: "2024-05-01" });
  });

  it("reports the precise timestamp for minute candles", () => {
    expect(
      pricingPriceMoment({
        ...txWithTags([]),
        pricingGranularity: "minute",
        pricingTimestamp: "2024-05-01T00:02:00Z",
      }),
    ).toEqual({ label: "Price timestamp", value: "2024-05-01 00:02" });
  });
});
