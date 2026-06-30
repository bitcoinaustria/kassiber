import type { ParseKeys } from "i18next";

import type { JournalEventItem } from "./TransactionDetailSheetParts";
import type { TransactionFlow } from "./model";

type TaxEffectState =
  | "pending"
  | "acquisition"
  | "income"
  | "disposal"
  | "transfer";
export type TaxTranslationKey = ParseKeys<["transactions"]>;

type TransactionTaxEffect = {
  state: TaxEffectState;
  costBasisEur: number | null;
  proceedsEur: number | null;
  gainLossEur: number | null;
  costBasisLabelKey: TaxTranslationKey;
  proceedsLabelKey: TaxTranslationKey;
  gainLossLabelKey: TaxTranslationKey;
  costBasisFallbackKey?: TaxTranslationKey;
  proceedsFallbackKey?: TaxTranslationKey;
  gainLossFallbackKey?: TaxTranslationKey;
};

const ACQUISITION_ENTRY_TYPES = new Set(["acquisition"]);
const INCOME_ENTRY_TYPES = new Set(["income"]);
const DISPOSAL_ENTRY_TYPES = new Set([
  "disposal",
  "fee",
  "transfer_fee",
  "neutral_swap",
]);
const TRANSFER_ENTRY_TYPES = new Set(["transfer_out", "transfer_in"]);

function numberOrZero(value: number | null | undefined) {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function sumJournalValues(
  events: JournalEventItem[],
  keys: Array<
    keyof Pick<
      JournalEventItem,
      "costBasisEur" | "proceedsEur" | "gainLossEur" | "fiatValueEur"
    >
  >,
) {
  return events.reduce((sum, event) => {
    for (const key of keys) {
      const value = event[key];
      if (typeof value === "number" && Number.isFinite(value)) {
        return sum + value;
      }
    }
    return sum;
  }, 0);
}

export function summarizeTransactionTaxEffect(
  events: JournalEventItem[],
  flow: TransactionFlow,
): TransactionTaxEffect {
  if (!events.length) {
    return {
      state: "pending",
      costBasisEur: null,
      proceedsEur: null,
      gainLossEur: null,
      costBasisLabelKey: "tax.costBasis",
      proceedsLabelKey: "tax.proceeds",
      gainLossLabelKey: "tax.gainLoss",
      costBasisFallbackKey: "tax.journalPending",
      proceedsFallbackKey: "tax.journalPending",
      gainLossFallbackKey: "tax.journalPending",
    };
  }

  const transferEvents = events.filter((event) =>
    TRANSFER_ENTRY_TYPES.has(event.entryType),
  );
  if (transferEvents.length || flow === "transfer") {
    return {
      state: "transfer",
      costBasisEur: null,
      proceedsEur: null,
      gainLossEur: null,
      costBasisLabelKey: "tax.basisTreatment",
      proceedsLabelKey: "tax.proceeds",
      gainLossLabelKey: "tax.gainLoss",
      costBasisFallbackKey: "tax.basisCarriedForward",
      proceedsFallbackKey: "tax.noDisposal",
      gainLossFallbackKey: "tax.noRealization",
    };
  }

  const disposalEvents = events.filter((event) =>
    DISPOSAL_ENTRY_TYPES.has(event.entryType),
  );
  if (disposalEvents.length) {
    return {
      state: "disposal",
      costBasisEur: sumJournalValues(disposalEvents, ["costBasisEur"]),
      proceedsEur: sumJournalValues(disposalEvents, [
        "proceedsEur",
        "fiatValueEur",
      ]),
      gainLossEur: sumJournalValues(disposalEvents, ["gainLossEur"]),
      costBasisLabelKey: "tax.costBasis",
      proceedsLabelKey: "tax.proceeds",
      gainLossLabelKey: "tax.gainLoss",
    };
  }

  const incomeEvents = events.filter((event) =>
    INCOME_ENTRY_TYPES.has(event.entryType),
  );
  if (incomeEvents.length) {
    return {
      state: "income",
      costBasisEur: sumJournalValues(incomeEvents, ["costBasisEur"]),
      proceedsEur: sumJournalValues(incomeEvents, [
        "proceedsEur",
        "fiatValueEur",
      ]),
      gainLossEur: sumJournalValues(incomeEvents, ["gainLossEur"]),
      costBasisLabelKey: "tax.costBasis",
      proceedsLabelKey: "tax.incomeRecognized",
      gainLossLabelKey: "tax.taxableIncome",
    };
  }

  const acquisitionEvents = events.filter(
    (event) =>
      ACQUISITION_ENTRY_TYPES.has(event.entryType) ||
      numberOrZero(event.quantity) > 0,
  );
  if (acquisitionEvents.length) {
    return {
      state: "acquisition",
      costBasisEur: sumJournalValues(acquisitionEvents, ["fiatValueEur"]),
      proceedsEur: null,
      gainLossEur: null,
      costBasisLabelKey: "tax.basisAdded",
      proceedsLabelKey: "tax.proceeds",
      gainLossLabelKey: "tax.gainLoss",
      proceedsFallbackKey: "tax.noDisposal",
      gainLossFallbackKey: "tax.notRealized",
    };
  }

  return {
    state: "pending",
    costBasisEur: null,
    proceedsEur: null,
    gainLossEur: null,
    costBasisLabelKey: "tax.costBasis",
    proceedsLabelKey: "tax.proceeds",
    gainLossLabelKey: "tax.gainLoss",
    costBasisFallbackKey: "tax.journalPending",
    proceedsFallbackKey: "tax.journalPending",
    gainLossFallbackKey: "tax.journalPending",
  };
}
