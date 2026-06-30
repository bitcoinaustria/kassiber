import { describe, expect, it } from "vitest";

import type { JournalEventItem } from "./TransactionDetailSheetParts";
import { summarizeTransactionTaxEffect } from "./TransactionTaxModel";

function journalEvent(
  overrides: Partial<JournalEventItem> = {},
): JournalEventItem {
  return {
    id: "journal-entry-1",
    entryType: "acquisition",
    asset: "BTC",
    quantity: 0.1,
    fiatValueEur: 1_000,
    ...overrides,
  };
}

describe("summarizeTransactionTaxEffect", () => {
  it("shows pending journal values when no journal rows are available", () => {
    expect(summarizeTransactionTaxEffect([], "incoming")).toMatchObject({
      state: "pending",
      costBasisEur: null,
      proceedsEur: null,
      gainLossEur: null,
      costBasisFallbackKey: "tax.journalPending",
      proceedsFallbackKey: "tax.journalPending",
      gainLossFallbackKey: "tax.journalPending",
    });
  });

  it("uses acquisition fair market value as newly added basis", () => {
    expect(
      summarizeTransactionTaxEffect(
        [journalEvent({ entryType: "income", fiatValueEur: 250 })],
        "incoming",
      ),
    ).toMatchObject({
      state: "acquisition",
      costBasisEur: 250,
      proceedsEur: null,
      gainLossEur: null,
      costBasisLabelKey: "tax.basisAdded",
      proceedsFallbackKey: "tax.noDisposal",
      gainLossFallbackKey: "tax.notRealized",
    });
  });

  it("uses RP2 cost basis, proceeds, and realized gain for disposals", () => {
    expect(
      summarizeTransactionTaxEffect(
        [
          journalEvent({
            entryType: "disposal",
            fiatValueEur: 9_000,
            costBasisEur: 7_000,
            proceedsEur: 9_000,
            gainLossEur: 2_000,
            quantity: -0.1,
          }),
        ],
        "outgoing",
      ),
    ).toMatchObject({
      state: "disposal",
      costBasisEur: 7_000,
      proceedsEur: 9_000,
      gainLossEur: 2_000,
    });
  });

  it("keeps own-wallet transfers as carrying-value treatment", () => {
    expect(
      summarizeTransactionTaxEffect(
        [
          journalEvent({
            entryType: "transfer_out",
            fiatValueEur: 0,
            quantity: -0.1,
          }),
          journalEvent({
            entryType: "transfer_in",
            fiatValueEur: 0,
            quantity: 0.1,
          }),
        ],
        "transfer",
      ),
    ).toMatchObject({
      state: "transfer",
      costBasisEur: null,
      proceedsEur: null,
      gainLossEur: null,
      costBasisLabelKey: "tax.basisTreatment",
      costBasisFallbackKey: "tax.basisCarriedForward",
      proceedsFallbackKey: "tax.noDisposal",
      gainLossFallbackKey: "tax.noRealization",
    });
  });
});
