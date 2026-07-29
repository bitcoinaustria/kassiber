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
    expect(summarizeTransactionTaxEffect([])).toMatchObject({
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
        [journalEvent({ entryType: "acquisition", fiatValueEur: 250 })],
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

  it("uses income rows as tax recognition, not acquisition basis", () => {
    expect(
      summarizeTransactionTaxEffect(
        [
          journalEvent({
            entryType: "acquisition",
            fiatValueEur: 250,
          }),
          journalEvent({
            id: "journal-entry-income",
            entryType: "income",
            fiatValueEur: 250,
            costBasisEur: 0,
            proceedsEur: 250,
            gainLossEur: 250,
          }),
        ],
      ),
    ).toMatchObject({
      state: "income",
      costBasisEur: 0,
      proceedsEur: 250,
      gainLossEur: 250,
      proceedsLabelKey: "tax.incomeRecognized",
      gainLossLabelKey: "tax.taxableIncome",
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

  it("keeps transfer treatment when a transfer also has a fee row", () => {
    expect(
      summarizeTransactionTaxEffect(
        [
          journalEvent({
            entryType: "transfer_out",
            fiatValueEur: 0,
            quantity: -0.101,
          }),
          journalEvent({
            entryType: "transfer_in",
            fiatValueEur: 0,
            quantity: 0.1,
          }),
          journalEvent({
            id: "journal-entry-fee",
            entryType: "transfer_fee",
            fiatValueEur: 65,
            costBasisEur: 60,
            proceedsEur: 65,
            gainLossEur: 5,
            quantity: -0.001,
          }),
        ],
      ),
    ).toMatchObject({
      state: "transfer",
      costBasisFallbackKey: "tax.basisCarriedForward",
      proceedsFallbackKey: "tax.noDisposal",
      gainLossFallbackKey: "tax.noRealization",
    });
  });

  it("reports the disposal when a split spend also has a transfer leg", () => {
    // A split spend books transfer_out for the owned slice and disposal for the
    // external slice under one journal transaction id. The transfer leg must not
    // hide the realized gain the journal, capital-gains report and RP2 all book.
    expect(
      summarizeTransactionTaxEffect([
        journalEvent({
          entryType: "transfer_out",
          fiatValueEur: 0,
          quantity: -0.3,
        }),
        journalEvent({
          entryType: "transfer_fee",
          costBasisEur: 5,
          proceedsEur: 5,
          gainLossEur: 0,
          quantity: -0.0001,
        }),
        journalEvent({
          entryType: "disposal",
          costBasisEur: 4_000,
          proceedsEur: 12_000,
          gainLossEur: 8_000,
          quantity: -0.2,
        }),
      ]),
    ).toMatchObject({
      state: "disposal",
      costBasisEur: 4_005,
      proceedsEur: 12_005,
      gainLossEur: 8_000,
    });
  });
});
