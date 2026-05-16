import { describe, expect, it } from "vitest";

import {
  draftForTransaction,
  formatCounterDisplayMoney,
  formatDisplayMoney,
  formatSignedDisplayMoney,
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
});

describe("money formatting", () => {
  it("renders missing fiat values as Null instead of zero", () => {
    expect(formatDisplayMoney(null, 0.01, "eur")).toBe("Null");
    expect(formatSignedDisplayMoney(null, 0.01, "eur")).toBe("Null");
    expect(formatCounterDisplayMoney(null, 0.01, "btc")).toBe("Null");
  });
});
