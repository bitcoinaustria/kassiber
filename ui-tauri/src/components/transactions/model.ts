import { CheckCircle2, Clock, RotateCcw, XCircle } from "lucide-react";
import type * as React from "react";

import { formatBtc, type Currency } from "@/lib/currency";
import {
  explorerTargetForTransaction,
  type ExplorerSettings,
} from "@/lib/explorer";
import { MOCK_OVERVIEW, type Tx } from "@/mocks/seed";

export type TransactionStatus = "completed" | "pending" | "failed" | "review";

export type TransactionDirection = "Receive" | "Send" | "Transfer";

export type TransactionFlow =
  | "incoming"
  | "outgoing"
  | "transfer"
  | "swap"
  | "layer-transition";

export type Transaction = {
  id: string;
  txnId: string;
  explorerId?: string;
  amount: number;
  amountBtc?: number;
  feeBtc?: number;
  feeEur?: number;
  asset?: string;
  rate?: number;
  note?: string;
  counterparty: string;
  counterpartyInitials: string;
  direction: TransactionDirection;
  flow?: TransactionFlow;
  wallet?: string;
  tag?: string;
  sourceType?: Tx["type"];
  paymentMethod: "On-chain" | "Exchange" | "Lightning" | "Liquid";
  date: string;
  status: TransactionStatus;
};

export type TransactionEditDraft = {
  label: string;
  tags: string[];
  note: string;
  atRegime: AustrianDraftRegime;
  atCategory: AustrianDraftCategory;
  pricingSourceKind: PricingSourceKind | null;
  pricingQuality: PricingQuality;
  manualCurrency: string;
  manualPrice: string;
  manualValue: string;
  manualSource: string;
  reviewStatus: TransactionStatus;
  taxable: boolean;
  excluded: boolean;
};

export type PricingSourceKind =
  | "generic_import"
  | "wallet_export"
  | "exchange_execution"
  | "btcpay_wallet_export"
  | "btcpay_invoice"
  | "btcpay_payment"
  | "manual_override"
  | "manual_rate_cache"
  | "fmv_provider";

export type PricingQuality =
  | "exact"
  | "provider_sample"
  | "coarse_fallback"
  | "missing";

export type PricingSelectionValue = PricingSourceKind | "missing";

export type AustrianDraftRegime = "neu" | "alt" | "outside";

export type AustrianDraftCategory =
  | "income_general"
  | "income_capital_yield"
  | "neu_gain"
  | "neu_loss"
  | "neu_swap"
  | "alt_spekulation"
  | "alt_taxfree"
  | "none";

export type DraftPricingOption = {
  value: PricingSelectionValue;
  sourceKind: PricingSourceKind | null;
  quality: PricingQuality;
  label: string;
  description?: string;
  external?: boolean;
};

export type NewTransactionDraft = {
  // Presenter/demo state only. Durable saves should map to core contract fields:
  // direction, pricing_source_kind/pricing_quality, at_regime, and at_category.
  sourceKind: "onchain" | "offchain" | "exchange" | "manual" | "internal";
  flow: TransactionFlow;
  occurredAt: string;
  confirmedAt: string;
  wallet: string;
  counterparty: string;
  transactionId: string;
  fromWallet: string;
  toWallet: string;
  fromExternal: string;
  toExternal: string;
  swapService: string;
  asset: string;
  amountSats: string;
  sendAsset: string;
  receiveAsset: string;
  sendAmountSats: string;
  receiveAmountSats: string;
  feeSats: string;
  network:
    | "Bitcoin"
    | "Lightning"
    | "Liquid"
    | "Ecash"
    | "Exchange"
    | "Other";
  pricingSourceKind: PricingSourceKind | null;
  pricingQuality: PricingQuality;
  fiatCurrency: string;
  pricePerBtc: string;
  totalValue: string;
  movementId: string;
  label: string;
  atRegime: AustrianDraftRegime;
  atCategory: AustrianDraftCategory;
  tags: string;
  note: string;
  reviewStatus: TransactionStatus;
  taxable: boolean;
  evidence: NewTransactionEvidence;
};

export type NewTransactionEvidence = {
  btcpayInvoiceId: string;
  exchangeCsvRow: string;
  swapId: string;
  txidOrPermalink: string;
  preimage: string;
};

export const SATS_PER_BTC = 100_000_000;

export const currencyFormatter = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "EUR",
});

export const compactCurrencyFormatter = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "EUR",
  notation: "compact",
  maximumFractionDigits: 0,
});

export const blurClass = (hidden: boolean) => (hidden ? "sensitive" : "");

export function formatInlineBtc(btc: number, precision = 8) {
  return `₿${Math.abs(btc).toFixed(precision)}`;
}

export function formatDisplayMoney(eur: number, btc: number, currency: Currency) {
  if (currency === "btc") return formatInlineBtc(btc);
  return currencyFormatter.format(eur);
}

export function formatSignedDisplayMoney(eur: number, btc: number, currency: Currency) {
  if (currency === "btc") return formatBtc(btc, { sign: true });
  const prefix = eur >= 0 ? "+ " : "− ";
  return `${prefix}${currencyFormatter.format(Math.abs(eur))}`;
}

export function formatCounterDisplayMoney(eur: number, btc: number, currency: Currency) {
  if (currency === "btc") return currencyFormatter.format(Math.abs(eur));
  return formatBtc(btc);
}

export function formatShortTxid(txid: string) {
  if (txid.length <= 18) return txid;
  return `${txid.slice(0, 10)}...${txid.slice(-6)}`;
}

function localDatetimeInputValue(date = new Date()) {
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000);
  return local.toISOString().slice(0, 16);
}

export function transactionBtc(txn: Transaction) {
  return txn.amountBtc ?? 0;
}

export function transactionFlow(txn: Transaction): TransactionFlow {
  if (txn.flow) return txn.flow;
  if (txn.tag?.toLowerCase().includes("swap")) return "swap";
  if (txn.direction === "Transfer") return "transfer";
  return txn.direction === "Receive" ? "incoming" : "outgoing";
}

export const transactionStatusStyles: Record<TransactionStatus, string> = {
  completed:
    "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  pending:
    "bg-amber-50 text-amber-700 ring-1 ring-inset ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-400 dark:ring-amber-400/20",
  failed:
    "bg-red-50 text-red-700 ring-1 ring-inset ring-red-600/20 dark:bg-red-900/30 dark:text-red-400 dark:ring-red-400/20",
  review:
    "bg-blue-50 text-blue-700 ring-1 ring-inset ring-blue-700/10 dark:bg-blue-900/30 dark:text-blue-400 dark:ring-blue-400/20",
};

export const transactionStatusIcons: Record<
  TransactionStatus,
  React.ComponentType<React.SVGProps<SVGSVGElement>>
> = {
  completed: CheckCircle2,
  pending: Clock,
  failed: XCircle,
  review: RotateCcw,
};

export const transactionStatusLabels: Record<TransactionStatus, string> = {
  completed: "Confirmed",
  pending: "Pending",
  failed: "Failed",
  review: "Needs review",
};

export const allTransactionStatuses: TransactionStatus[] = [
  "completed",
  "pending",
  "failed",
  "review",
];

export const allPaymentMethods = [
  "On-chain",
  "Exchange",
  "Lightning",
  "Liquid",
] as const;

export const transactionFlowLabels: Record<TransactionFlow, string> = {
  incoming: "Incoming",
  outgoing: "Outgoing",
  transfer: "Internal transfer",
  swap: "Swap",
  "layer-transition": "Layer transition",
};

export const transactionFlowStyles: Record<TransactionFlow, string> = {
  incoming:
    "border-emerald-600/20 bg-emerald-50 text-emerald-700 dark:bg-emerald-900/25 dark:text-emerald-300",
  outgoing:
    "border-red-600/20 bg-red-50 text-red-700 dark:bg-red-900/25 dark:text-red-300",
  transfer:
    "border-zinc-500/20 bg-zinc-50 text-zinc-700 dark:bg-zinc-800/70 dark:text-zinc-300",
  swap:
    "border-sky-600/20 bg-sky-50 text-sky-700 dark:bg-sky-900/25 dark:text-sky-300",
  "layer-transition":
    "border-teal-600/20 bg-teal-50 text-teal-700 dark:bg-teal-900/25 dark:text-teal-300",
};

export const allTransactionFlows: TransactionFlow[] = [
  "incoming",
  "outgoing",
  "transfer",
  "swap",
  "layer-transition",
];

export function explorerForTransaction(
  txn: Transaction,
  settings: ExplorerSettings,
) {
  if (txn.paymentMethod === "Liquid") {
    return explorerTargetForTransaction({
      txid: txn.explorerId,
      network: "liquid",
      settings,
    });
  }
  if (txn.paymentMethod === "On-chain") {
    return explorerTargetForTransaction({
      txid: txn.explorerId,
      network: "bitcoin",
      settings,
    });
  }
  return null;
}

export const classificationOptions = [
  "Unlabeled",
  "Income",
  "Expense",
  "Transfer",
  "Swap",
  "Fee",
  "Merchant payment",
  "Gift",
  "Review",
  "Other",
];

export const tagSuggestions = [
  "Revenue",
  "BTCPay",
  "needs invoice",
  "client ACME",
  "Hosting",
  "Capex",
  "Meals",
  "Bank fees",
  "Consolidation",
  "Liquid",
  "Lightning",
  "manual review",
  "accountant",
];

export const transactionPricingOptions: DraftPricingOption[] = [
  {
    value: "generic_import",
    sourceKind: "generic_import",
    quality: "exact",
    label: "Source price",
    description: "Use source-provided price",
  },
  {
    value: "fmv_provider",
    sourceKind: "fmv_provider",
    quality: "provider_sample",
    label: "FMV provider",
    description: "Use a sampled market rate",
  },
  {
    value: "manual_override",
    sourceKind: "manual_override",
    quality: "exact",
    label: "Manual override",
    description: "Use invoice or receipt evidence",
  },
  {
    value: "missing",
    sourceKind: null,
    quality: "missing",
    label: "Missing / review",
    description: "Keep in review queue",
  },
];

export const pricingSourceStyles: Record<PricingSelectionValue, string> = {
  generic_import:
    "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  wallet_export:
    "bg-indigo-50 text-indigo-700 ring-1 ring-inset ring-indigo-600/20 dark:bg-indigo-900/25 dark:text-indigo-300 dark:ring-indigo-400/20",
  exchange_execution:
    "bg-emerald-50 text-emerald-700 ring-1 ring-inset ring-emerald-600/20 dark:bg-emerald-900/30 dark:text-emerald-400 dark:ring-emerald-400/20",
  btcpay_wallet_export:
    "bg-orange-50 text-orange-700 ring-1 ring-inset ring-orange-600/20 dark:bg-orange-900/25 dark:text-orange-300 dark:ring-orange-400/20",
  btcpay_invoice:
    "bg-orange-50 text-orange-700 ring-1 ring-inset ring-orange-600/20 dark:bg-orange-900/25 dark:text-orange-300 dark:ring-orange-400/20",
  btcpay_payment:
    "bg-orange-50 text-orange-700 ring-1 ring-inset ring-orange-600/20 dark:bg-orange-900/25 dark:text-orange-300 dark:ring-orange-400/20",
  manual_rate_cache:
    "bg-sky-50 text-sky-700 ring-1 ring-inset ring-sky-600/20 dark:bg-sky-900/30 dark:text-sky-400 dark:ring-sky-400/20",
  manual_override:
    "bg-violet-50 text-violet-700 ring-1 ring-inset ring-violet-600/20 dark:bg-violet-900/30 dark:text-violet-300 dark:ring-violet-400/20",
  fmv_provider:
    "bg-sky-50 text-sky-700 ring-1 ring-inset ring-sky-600/20 dark:bg-sky-900/30 dark:text-sky-400 dark:ring-sky-400/20",
  missing:
    "bg-amber-50 text-amber-700 ring-1 ring-inset ring-amber-600/20 dark:bg-amber-900/30 dark:text-amber-400 dark:ring-amber-400/20",
};

export const newTransactionPricingOptions: DraftPricingOption[] = [
  {
    value: "manual_override",
    sourceKind: "manual_override",
    quality: "exact",
    label: "Manual entry",
    external: false,
  },
  {
    value: "fmv_provider",
    sourceKind: "fmv_provider",
    quality: "provider_sample",
    label: "FMV provider sample",
    external: true,
  },
  {
    value: "exchange_execution",
    sourceKind: "exchange_execution",
    quality: "exact",
    label: "Exchange execution",
    external: true,
  },
  {
    value: "btcpay_invoice",
    sourceKind: "btcpay_invoice",
    quality: "exact",
    label: "BTCPay invoice fiat",
    external: true,
  },
  {
    value: "wallet_export",
    sourceKind: "wallet_export",
    quality: "exact",
    label: "Wallet import exact fiat",
    external: true,
  },
  {
    value: "missing",
    sourceKind: null,
    quality: "missing",
    label: "Needs pricing",
    external: false,
  },
];

export const austrianTaxClassificationOptions: Array<{
  value: string;
  label: string;
  shortLabel: string;
  atRegime: AustrianDraftRegime;
  atCategory: AustrianDraftCategory;
  taxable: boolean;
}> = [
  {
    value: "neu:neu_gain",
    label: "§27b Neu taxable disposal",
    shortLabel: "Neu gain",
    atRegime: "neu",
    atCategory: "neu_gain",
    taxable: true,
  },
  {
    value: "neu:income_general",
    label: "§27b income receipt",
    shortLabel: "Income receipt",
    atRegime: "neu",
    atCategory: "income_general",
    taxable: true,
  },
  {
    value: "neu:neu_loss",
    label: "§27b disposal loss / fee review",
    shortLabel: "Neu loss",
    atRegime: "neu",
    atCategory: "neu_loss",
    taxable: true,
  },
  {
    value: "outside:none",
    label: "Own-wallet transfer / outside §27b",
    shortLabel: "Own-wallet transfer",
    atRegime: "outside",
    atCategory: "none",
    taxable: false,
  },
  {
    value: "neu:neu_swap",
    label: "§27b carrying-value swap / layer transition",
    shortLabel: "Neu swap",
    atRegime: "neu",
    atCategory: "neu_swap",
    taxable: false,
  },
  {
    value: "alt:alt_spekulation",
    label: "Altbestand within speculation period",
    shortLabel: "Alt taxable",
    atRegime: "alt",
    atCategory: "alt_spekulation",
    taxable: true,
  },
  {
    value: "alt:alt_taxfree",
    label: "Altbestand tax-free",
    shortLabel: "Alt tax-free",
    atRegime: "alt",
    atCategory: "alt_taxfree",
    taxable: false,
  },
];

export const newTransactionNetworkOptions: NewTransactionDraft["network"][] = [
  "Bitcoin",
  "Lightning",
  "Liquid",
  "Ecash",
  "Exchange",
  "Other",
];

export function sourceKindForNetwork(
  network: NewTransactionDraft["network"],
): NewTransactionDraft["sourceKind"] {
  if (network === "Bitcoin") return "onchain";
  if (network === "Exchange") return "exchange";
  if (network === "Other") return "manual";
  return "offchain";
}

export const mockNewTransactionWalletSourceOptions = [
  ...MOCK_OVERVIEW.connections.map((connection) => connection.label),
  "External",
];

export const newTransactionFlowOptions: Array<{
  value: TransactionFlow;
  label: string;
}> = [
  { value: "incoming", label: "Receive" },
  { value: "outgoing", label: "Send" },
  { value: "transfer", label: "Internal transfer" },
  { value: "swap", label: "Swap" },
  { value: "layer-transition", label: "Layer transition" },
];

export const mockNewTransactionMovementCandidates = [
  {
    id: "movement-ln-channel-open",
    label: "Join channel open · Home Node",
    detail: "tx8 · same wallet · 96% confidence",
  },
  {
    id: "movement-liquid-peg",
    label: "Join Liquid peg transition",
    detail: "LBTC leg 8 min apart · 89% confidence",
  },
  {
    id: "movement-boltz-swap",
    label: "Join Boltz swap pair",
    detail: "payment hash overlap · 82% confidence",
  },
];

export function draftForTransaction(txn: Transaction): TransactionEditDraft {
  const flow = transactionFlow(txn);
  const initialTags = splitDraftTags(txn.tag || "");
  const defaultTaxClassification = nextTaxClassificationForFlow(flow);
  return {
    label:
      classificationOptions.find((option) => initialTags.includes(option)) ??
      (flow === "incoming"
        ? "Income"
        : flow === "outgoing"
          ? "Expense"
          : flow === "swap"
            ? "Swap"
            : flow === "transfer"
              ? "Transfer"
              : "Unlabeled"),
    tags: uniqueTags(
      initialTags.filter((tag) => !classificationOptions.includes(tag)),
    ),
    note: txn.note || "",
    atRegime: defaultTaxClassification.atRegime,
    atCategory: defaultTaxClassification.atCategory,
    pricingSourceKind: txn.rate ? "generic_import" : null,
    pricingQuality: txn.rate ? "exact" : "missing",
    manualCurrency: "EUR",
    manualPrice: txn.rate ? String(txn.rate) : "",
    manualValue: txn.amount ? String(txn.amount) : "",
    manualSource: "",
    reviewStatus: txn.status,
    taxable: flow !== "transfer",
    excluded: false,
  };
}

export function splitDraftTags(tags: string) {
  return tags
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean);
}

export function uniqueTags(tags: string[]) {
  return Array.from(new Set(tags.map((tag) => tag.trim()).filter(Boolean)));
}

export function pricingSelectionValue(
  sourceKind: PricingSourceKind | null,
  quality: PricingQuality,
): PricingSelectionValue {
  return quality === "missing" || !sourceKind ? "missing" : sourceKind;
}

export function pricingSourceLabel(
  sourceKind: PricingSourceKind | null,
  quality: PricingQuality,
  options = transactionPricingOptions,
) {
  const value = pricingSelectionValue(sourceKind, quality);
  return options.find((option) => option.value === value)?.label ?? value;
}

export function pricingOptionForValue(
  value: PricingSelectionValue,
  options = transactionPricingOptions,
) {
  return options.find((option) => option.value === value) ?? options[0];
}

export function isExternalPricingSource(
  sourceKind: PricingSourceKind | null,
  quality: PricingQuality,
  options = newTransactionPricingOptions,
) {
  const value = pricingSelectionValue(sourceKind, quality);
  return Boolean(
    options.find((option) => option.value === value && option.external),
  );
}

export function austrianSelectionValue(
  atRegime: AustrianDraftRegime,
  atCategory: AustrianDraftCategory,
) {
  return `${atRegime}:${atCategory}`;
}

export function austrianTaxClassificationFor(
  atRegime: AustrianDraftRegime,
  atCategory: AustrianDraftCategory,
) {
  return (
    austrianTaxClassificationOptions.find(
      (option) =>
        option.atRegime === atRegime && option.atCategory === atCategory,
    ) ?? austrianTaxClassificationOptions[0]
  );
}

export function austrianTaxClassificationForValue(value: string) {
  return (
    austrianTaxClassificationOptions.find((option) => option.value === value) ??
    austrianTaxClassificationOptions[0]
  );
}

export function parseManualDecimal(value: string) {
  const normalized = value.trim().replace(",", ".");
  if (!normalized) return null;
  const parsed = Number(normalized);
  return Number.isFinite(parsed) ? parsed : null;
}

export function formatManualFiat(value: number) {
  if (!Number.isFinite(value)) return "";
  return value.toFixed(2);
}

export function formatDraftFiat(value: number, currencyCode: NewTransactionDraft["fiatCurrency"]) {
  if (!Number.isFinite(value)) return "-";
  return `${currencyFormatter.format(value)} ${currencyCode}`;
}

export function formatManualPrice(value: number) {
  if (!Number.isFinite(value)) return "";
  return value.toFixed(2);
}

export function formatBtcAmount(value: number, precision = 8) {
  return `${value.toFixed(precision)} BTC`;
}

export function formatAssetAmount(value: number, asset: string, precision = 8) {
  return `${value.toFixed(precision)} ${asset || "BTC"}`;
}

export function formatFee(txn: Transaction, currency: Currency) {
  const feeBtc = txn.feeBtc ?? 0;
  if (!feeBtc) return "-";
  if (currency === "btc") return formatBtcAmount(feeBtc);
  return currencyFormatter.format(txn.feeEur ?? 0);
}

export function createNewTransactionDraft(): NewTransactionDraft {
  return {
    sourceKind: "onchain",
    flow: "incoming",
    occurredAt: localDatetimeInputValue(),
    confirmedAt: "",
    wallet: "Cold Storage",
    counterparty: "",
    transactionId: "",
    fromWallet: "Cold Storage",
    toWallet: "Cold Storage",
    fromExternal: "",
    toExternal: "",
    swapService: "",
    asset: "BTC",
    amountSats: "",
    sendAsset: "BTC",
    receiveAsset: "BTC",
    sendAmountSats: "",
    receiveAmountSats: "",
    feeSats: "",
    network: "Bitcoin",
    pricingSourceKind: "manual_override",
    pricingQuality: "exact",
    fiatCurrency: "EUR",
    pricePerBtc: "",
    totalValue: "",
    movementId: "",
    label: "Income",
    atRegime: "neu",
    atCategory: "income_general",
    tags: "",
    note: "",
    reviewStatus: "review",
    taxable: true,
    evidence: {
      btcpayInvoiceId: "",
      exchangeCsvRow: "",
      swapId: "",
      txidOrPermalink: "",
      preimage: "",
    },
  };
}

export function parseSatsInput(value: string) {
  const normalized = value.trim().replace(/[,_\s]/g, "");
  if (!normalized) return null;
  const parsed = Number(normalized);
  return Number.isFinite(parsed) ? Math.trunc(Math.abs(parsed)) : null;
}

export function btcFromSatsInput(value: string) {
  const sats = parseSatsInput(value);
  return sats === null ? null : sats / SATS_PER_BTC;
}

export function nextTaxClassificationForFlow(flow: TransactionFlow) {
  if (flow === "incoming") {
    return austrianTaxClassificationFor("neu", "income_general");
  }
  if (flow === "transfer") {
    return austrianTaxClassificationFor("outside", "none");
  }
  if (flow === "swap" || flow === "layer-transition") {
    return austrianTaxClassificationFor("neu", "neu_swap");
  }
  return austrianTaxClassificationFor("neu", "neu_gain");
}

export function nextLabelForFlow(flow: TransactionFlow) {
  if (flow === "incoming") return "Income";
  if (flow === "transfer") return "Transfer";
  if (flow === "swap") return "Swap";
  if (flow === "layer-transition") return "Transfer";
  return "Expense";
}

export function isTwoLegNewTransactionFlow(flow: TransactionFlow) {
  return flow === "swap" || flow === "layer-transition";
}

export function showConfirmedAtForDraft(draft: NewTransactionDraft) {
  return draft.network === "Bitcoin" || draft.network === "Liquid";
}

export function showSingleAssetForDraft(draft: NewTransactionDraft) {
  return (
    !isTwoLegNewTransactionFlow(draft.flow) &&
    (draft.network === "Exchange" || draft.network === "Other")
  );
}

export function inferredAssetForDraft(draft: NewTransactionDraft) {
  if (showSingleAssetForDraft(draft)) return draft.asset || "BTC";
  if (draft.network === "Liquid") return "LBTC";
  return "BTC";
}

export type PricingDraftField =
  | "amountSats"
  | "sendAmountSats"
  | "receiveAmountSats"
  | "pricePerBtc"
  | "totalValue";

export function pricingAmountSatsForDraft(
  draft: NewTransactionDraft,
  preferredAmountField?: Extract<
    PricingDraftField,
    "amountSats" | "sendAmountSats" | "receiveAmountSats"
  >,
): string {
  if (preferredAmountField) {
    return draft[preferredAmountField] || pricingAmountSatsForDraft(draft);
  }
  if (isTwoLegNewTransactionFlow(draft.flow)) {
    return draft.receiveAmountSats || draft.sendAmountSats || draft.amountSats;
  }
  return draft.amountSats;
}

export function calculateNewTransactionPricing(
  draft: NewTransactionDraft,
  changed: PricingDraftField,
): NewTransactionDraft {
  const preferredAmountField =
    changed === "amountSats" ||
    changed === "sendAmountSats" ||
    changed === "receiveAmountSats"
      ? changed
      : undefined;
  const btc = btcFromSatsInput(pricingAmountSatsForDraft(draft, preferredAmountField));
  const price = parseManualDecimal(draft.pricePerBtc);
  const total = parseManualDecimal(draft.totalValue);
  const amountChanged =
    changed === "amountSats" ||
    changed === "sendAmountSats" ||
    changed === "receiveAmountSats";

  if (!btc || btc <= 0) return draft;

  if ((amountChanged || changed === "pricePerBtc") && price !== null) {
    return {
      ...draft,
      totalValue: formatManualFiat(btc * price),
    };
  }
  if ((amountChanged || changed === "totalValue") && total !== null) {
    return {
      ...draft,
      pricePerBtc: formatManualPrice(total / btc),
    };
  }
  return draft;
}

export function signedNewTransactionBtc(draft: NewTransactionDraft) {
  if (isTwoLegNewTransactionFlow(draft.flow)) {
    const received = btcFromSatsInput(draft.receiveAmountSats) ?? 0;
    const sent = btcFromSatsInput(draft.sendAmountSats) ?? 0;
    return received - sent;
  }
  const btc = btcFromSatsInput(draft.amountSats) ?? 0;
  if (draft.flow === "outgoing") return -btc;
  if (draft.flow === "transfer") return 0;
  return btc;
}

export function copyText(value: string | undefined) {
  if (!value || typeof navigator === "undefined") return;
  navigator.clipboard?.writeText(value);
}
