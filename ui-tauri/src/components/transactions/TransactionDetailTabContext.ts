import * as React from "react";

import type { Currency } from "@/lib/currency";

import {
  austrianTaxClassificationFor,
  pricingPriceMoment,
  transactionFlow,
  type Transaction,
  type TransactionEditDraft,
} from "./model";
import type {
  CommercialContextData,
  DirtyMap,
  JournalEventItem,
} from "./TransactionDetailSheetParts";
import type { TransactionGraphPayload } from "./TransactionGraphTab";

export type UpdateTransactionDraft = <K extends keyof TransactionEditDraft>(
  key: K,
  value: TransactionEditDraft[K],
) => void;

export type TransactionDetailTabContext = {
  transaction: Transaction;
  localDraft: TransactionEditDraft;
  dirty: DirtyMap;
  dirtyLabel?: boolean;
  dirtyTags?: boolean;
  dirtyNote?: boolean;
  dirtyPricing: boolean;
  dirtyExcluded?: boolean;
  dirtyReviewTax: boolean;
  hideSensitive: boolean;
  currency: Currency;
  transactionDisplayId: string;
  feeBtc: number;
  commercialContext?: CommercialContextData;
  commercialContextLoading?: boolean;
  showSourceExternalId: boolean;
  updateDraft: UpdateTransactionDraft;
  tags: string[];
  tagInput: string;
  setTagInput: React.Dispatch<React.SetStateAction<string>>;
  tagInputRef: React.RefObject<HTMLInputElement | null>;
  addTag: (rawTag: string) => void;
  removeTag: (tag: string) => void;
  availableTagSuggestions: string[];
  amountBtc: number;
  pricingValue: string;
  updateManualPrice: (rawPrice: string) => void;
  updateManualValue: (rawValue: string) => void;
  manualPriceRef: React.RefObject<HTMLInputElement | null>;
  hasCacheProvenance: boolean;
  isCoarsePricing: boolean;
  isProviderSamplePricing: boolean;
  isExactPricing: boolean;
  isPricingMissing: boolean;
  pricePoint: ReturnType<typeof pricingPriceMoment>;
  nowRate?: number | null;
  onOpenMarketDataSettings?: () => void;
  openMarketDataSettings: () => void;
  chooseExactManualPrice: () => void;
  flow: ReturnType<typeof transactionFlow>;
  taxNarrative: string;
  taxClassification: ReturnType<typeof austrianTaxClassificationFor>;
  valueAtTimeEur: number | null;
  pair: Transaction["pair"];
  onUnpair?: (pairId: string) => void | Promise<void>;
  isUnpairing?: boolean;
  journalEvents: JournalEventItem[];
  balanceCurrency: Currency;
  setBalanceCurrency: React.Dispatch<React.SetStateAction<Currency>>;
  impactDirection: number;
  principalImpactBtc: number;
  principalImpactEur: number | null;
  feeImpactBtc: number;
  feeImpactEur: number | null;
  netImpactBtc: number;
  netImpactEur: number | null;
  graphData?: TransactionGraphPayload;
  graphLoading?: boolean;
  graphError?: string | null;
};
