import type { ParseKeys } from "i18next";
import {
  ArrowRight,
  Coins,
  Link2Off,
  RotateCcw,
  Save,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Sheet,
  SheetContent,
  SheetFooter,
} from "@/components/ui/sheet";
import {
  Tabs,
  TabsList,
  TabsTrigger,
} from "@/components/ui/tabs";
import { TooltipProvider } from "@/components/ui/tooltip";
import { useDaemon } from "@/daemon/client";
import { type Currency } from "@/lib/currency";
import type { ExplorerSettings } from "@/lib/explorer";
import type {
  HistoryRevertTarget,
  TransactionHistoryEvent,
  TransactionHistoryStaleSummary,
} from "@/lib/transactionHistory";

import {
  austrianTaxClassificationFor,
  classificationOptionLabelKeys,
  currencyFormatter,
  explorerForTransaction,
  formatBtcAmount,
  formatManualFiat,
  formatManualPrice,
  parseManualDecimal,
  pricingPriceMoment,
  pricingSelectionValue,
  pricingSourceLabel,
  shouldShowSourceExternalId,
  tagSuggestions,
  transactionBtc,
  transactionFlow,
  transactionStatusLabels,
  type LoanMarkTarget,
  type LoanMark,
  type Transaction,
  type TransactionEditDraft,
  uniqueTags,
} from "./model";
import { TransactionSplitPayoutCard } from "./TransactionSplitPayoutCard";

import {
  DirtyDot,
  QuarantineBanner,
  balanceImpactDirection,
  confirmationsLabel,
  countDirty,
  diffDraft,
  rateChangePct,
  type AttachmentItem,
  type ChecklistItem,
  type CommercialContextData,
  type JournalEventItem,
  type TimelineStep,
} from "./TransactionDetailSheetParts";
export type { AttachmentItem, CommercialContextData, JournalEventItem } from "./TransactionDetailSheetParts";
import { TransactionDetailHeader } from "./TransactionDetailHeader";
import { TransactionDetailRightRail } from "./TransactionDetailRightRail";
import {
  TransactionClassifyTab,
  TransactionDetailsTab,
  TransactionLedgerTab,
  TransactionLinkedTab,
  TransactionPricingTab,
  TransactionTaxTab,
  transactionGraphLookupArgs,
  type TransactionGraphPayload,
  type TransactionDetailTabContext,
} from "./TransactionDetailSheetTabs";

// ─── main component ────────────────────────────────────────────────────

export function TransactionDetailSheet({
  transaction,
  draft,
  initialTab,
  hideSensitive,
  currency,
  explorerSettings,
  isSaving,
  saveError,
  quarantineReasonOverride,
  nowRate,
  attachments,
  journalEvents = [],
  commercialContext,
  commercialContextLoading,
  historyEvents,
  historyStale,
  historyLoading,
  isRevertingHistory,
  onAddAttachmentFiles,
  onAddAttachmentLinks,
  onReuseEvidence,
  onOpenAttachment,
  onRenameAttachment,
  onRemoveAttachment,
  onUnpair,
  isUnpairing,
  onOpenPairingReview,
  onOpenMarketDataSettings,
  onRevertHistory,
  onProcessJournals,
  isProcessingJournals,
  loanRole,
  loanMark,
  linkedLoanMarks,
  loanLinkCandidates,
  isLoanMarking,
  isLoanLinking,
  onMarkLoan,
  onUnmarkLoan,
  onLinkLoan,
  onOpenChange,
  onOpenExplorer,
  onSave,
  onSaveAndNext,
  hasNext,
}: {
  transaction: Transaction | null;
  draft: TransactionEditDraft | null;
  initialTab: string;
  hideSensitive: boolean;
  currency: Currency;
  explorerSettings: ExplorerSettings;
  isSaving?: boolean;
  saveError?: string | null;
  quarantineReasonOverride?: string | null;
  nowRate?: number | null;
  attachments?: AttachmentItem[];
  journalEvents?: JournalEventItem[];
  commercialContext?: CommercialContextData;
  commercialContextLoading?: boolean;
  historyEvents?: TransactionHistoryEvent[];
  historyStale?: TransactionHistoryStaleSummary;
  historyLoading?: boolean;
  isRevertingHistory?: boolean;
  onAddAttachmentFiles?: (paths: string[]) => void | Promise<void>;
  onAddAttachmentLinks?: (urls: string[]) => void | Promise<void>;
  onReuseEvidence?: () => void;
  onOpenAttachment?: (item: AttachmentItem) => void;
  onRenameAttachment?: (
    item: AttachmentItem,
    label: string,
  ) => void | Promise<void>;
  onRemoveAttachment?: (item: AttachmentItem) => void;
  onUnpair?: (pairId: string) => void | Promise<void>;
  isUnpairing?: boolean;
  onOpenPairingReview?: () => void;
  onOpenMarketDataSettings?: () => void;
  onRevertHistory?: (target: HistoryRevertTarget) => void | Promise<void>;
  onProcessJournals?: () => void;
  isProcessingJournals?: boolean;
  loanRole?: string | null;
  loanMark?: LoanMark | null;
  linkedLoanMarks?: LoanMark[];
  loanLinkCandidates?: LoanMark[];
  isLoanMarking?: boolean;
  isLoanLinking?: boolean;
  onMarkLoan?: (transaction: Transaction, as: LoanMarkTarget) => void | Promise<void>;
  onUnmarkLoan?: (transaction: Transaction) => void | Promise<void>;
  onLinkLoan?: (transaction: Transaction, targetTransactionId: string) => void | Promise<void>;
  onOpenChange: (open: boolean) => void;
  onOpenExplorer: (transaction: Transaction) => void;
  onSave: (
    transactionId: string,
    draft: TransactionEditDraft,
  ) => void | Promise<void>;
  onSaveAndNext?: (
    transactionId: string,
    draft: TransactionEditDraft,
  ) => void | Promise<void>;
  hasNext?: boolean;
}) {
  const { t } = useTranslation(["transactions", "common"]);
  const visibleInitialTab = initialTab === "graph" ? "details" : initialTab;
  const [activeTab, setActiveTab] = React.useState(visibleInitialTab);
  const [localDraft, setLocalDraft] =
    React.useState<TransactionEditDraft | null>(draft);
  const [originalDraft, setOriginalDraft] =
    React.useState<TransactionEditDraft | null>(draft);
  const [tagInput, setTagInput] = React.useState("");
  const [balanceCurrency, setBalanceCurrency] =
    React.useState<Currency>(currency);
  const manualPriceRef = React.useRef<HTMLInputElement | null>(null);
  const graphQuery = useDaemon<TransactionGraphPayload>(
    "ui.transactions.graph",
    transactionGraphLookupArgs(transaction),
    { enabled: Boolean(transaction) },
  );
  React.useEffect(() => {
    setActiveTab(visibleInitialTab);
  }, [visibleInitialTab, transaction?.id]);

  React.useEffect(() => {
    setLocalDraft(draft);
    setOriginalDraft(draft);
    setTagInput("");
    setBalanceCurrency(currency);
  }, [currency, draft, transaction?.id]);

  const tagInputRef = React.useRef<HTMLInputElement | null>(null);
  const dirty = React.useMemo(
    () => (localDraft && originalDraft ? diffDraft(localDraft, originalDraft) : {}),
    [localDraft, originalDraft],
  );
  const dirtyCount = countDirty(dirty);

  const updateDraft = React.useCallback(
    <K extends keyof TransactionEditDraft>(
      key: K,
      value: TransactionEditDraft[K],
    ) => {
      setLocalDraft((current) =>
        current ? { ...current, [key]: value } : current,
      );
    },
    [],
  );

  // Keyboard shortcuts: 1-6 tabs, Cmd/Ctrl+S save, Esc close, e excluded, t focus tag.
  // Suppress shortcuts while focus is inside another modal (e.g. AttachLinksDialog)
  // so dialog keys don't reach back into the underlying sheet.
  React.useEffect(() => {
    if (!transaction || !localDraft) return;
    const handler = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const isTyping =
        target?.tagName === "INPUT" ||
        target?.tagName === "TEXTAREA" ||
        target?.isContentEditable;
      const insideNestedDialog = Boolean(
        target?.closest('[data-slot="dialog-content"]'),
      );
      if (insideNestedDialog) return;
      if ((event.metaKey || event.ctrlKey) && event.key === "s") {
        event.preventDefault();
        if (dirtyCount > 0 && !isSaving) {
          void onSave(transaction.id, localDraft);
        }
        return;
      }
      if (event.key === "Escape" && !isTyping) {
        onOpenChange(false);
        return;
      }
      if (isTyping) return;
      if (["1", "2", "3", "4", "5", "6"].includes(event.key)) {
        const order = ["details", "classify", "pricing", "tax", "linked", "ledger"];
        const next = order[Number(event.key) - 1];
        if (next) setActiveTab(next);
        return;
      }
      if (event.key === "e") {
        updateDraft("excluded", !localDraft.excluded);
        return;
      }
      if (event.key === "t") {
        event.preventDefault();
        setActiveTab("classify");
        setTimeout(() => tagInputRef.current?.focus(), 0);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [transaction, localDraft, dirtyCount, isSaving, onOpenChange, onSave, updateDraft]);

  if (!transaction || !localDraft) return null;

  const flow = transactionFlow(transaction);
  const canMarkLoan = Boolean(onMarkLoan || onUnmarkLoan);
  const loanActionDisabled = Boolean(isSaving || isLoanMarking);
  const explorer = explorerForTransaction(transaction, explorerSettings);
  const transactionDisplayId = transaction.explorerId ?? transaction.txnId;
  const showSourceExternalId = shouldShowSourceExternalId(transaction);
  const amountBtc = transactionBtc(transaction);
  const feeBtc = transaction.feeBtc ?? 0;
  const feeEur = transaction.feeEur ?? null;
  const impactDirection = balanceImpactDirection(transaction, flow);
  const isFeeOnly = transaction.sourceType === "Fee";
  const principalImpactBtc = isFeeOnly ? 0 : impactDirection * amountBtc;
  const principalImpactEur =
    transaction.amount === null
      ? null
      : isFeeOnly
        ? 0
        : impactDirection * transaction.amount;
  const feeImpactBtc = feeBtc ? -feeBtc : 0;
  const feeImpactEur = feeBtc ? (feeEur === null ? null : -feeEur) : 0;
  const netImpactBtc = principalImpactBtc + feeImpactBtc;
  const netImpactEur =
    principalImpactEur === null || feeImpactEur === null
      ? null
      : principalImpactEur + feeImpactEur;
  const pair = transaction.pair;
  const signedPrefix =
    flow === "incoming" ? "+" : flow === "outgoing" ? "-" : "";
  const tags = localDraft.tags;
  const taxClassification = austrianTaxClassificationFor(
    localDraft.atRegime,
    localDraft.atCategory,
  );
  const pricingValue = pricingSelectionValue(
    localDraft.pricingSourceKind,
    localDraft.pricingQuality,
  );
  const hasCacheProvenance = Boolean(
    transaction.pricingProvider || transaction.pricingPair,
  );
  const pricePoint = pricingPriceMoment(transaction);
  const isExactPricing = localDraft.pricingQuality === "exact";
  const isCoarsePricing = localDraft.pricingQuality === "coarse_fallback";
  const isProviderSamplePricing = localDraft.pricingQuality === "provider_sample";
  const sourceName = transaction.wallet || transaction.paymentMethod;
  const sourceType = transaction.sourceType ?? transaction.paymentMethod;
  const settlementLabel = t(transactionStatusLabels[transaction.status]);
  const valueAtTimeEur = transaction.amount;
  const valueNowEur =
    nowRate && amountBtc ? nowRate * amountBtc * (impactDirection || 1) : null;
  const pricedChange = rateChangePct(nowRate ?? null, transaction.rate ?? null);
  const isPricingMissing =
    localDraft.pricingSourceKind === null ||
    localDraft.pricingQuality === "missing" ||
    transaction.amount === null;
  const isLabeled =
    localDraft.label !== "Unlabeled" && localDraft.label.trim().length > 0;
  const quarantineReason =
    quarantineReasonOverride ?? transaction.quarantineReason ?? null;
  const quarantineReasonCode = quarantineReason?.toLowerCase() ?? "";
  const isSyncQuarantine = quarantineReasonCode.includes(
    "ownership_transfer_amount_mismatch",
  );
  const isBasisQuarantine =
    quarantineReasonCode.includes("basis") ||
    quarantineReasonCode.includes("lot") ||
    quarantineReasonCode.includes("insufficient");
  const isTransferQuarantine =
    quarantineReasonCode.includes("ownership_transfer") ||
    quarantineReasonCode.includes("transfer") ||
    quarantineReasonCode.includes("pair") ||
    quarantineReasonCode.includes("swap");
  const isSplitTransferQuarantine =
    quarantineReasonCode.includes("transfer_fee_implausible");
  const quarantineTargetTab = isSplitTransferQuarantine
    ? "details"
    : isSyncQuarantine
      ? "details"
      : isTransferQuarantine
        ? "linked"
        : isBasisQuarantine
          ? "tax"
          : "pricing";
  const hasJournalQuarantine = Boolean(quarantineReason) && !localDraft.excluded;
  const hasPricingBlocker = isPricingMissing && !localDraft.excluded;
  const suppressPricingCacheWarning =
    hasJournalQuarantine && quarantineTargetTab === "pricing";
  const suppressBasisQuarantineWarning =
    hasJournalQuarantine && quarantineTargetTab === "tax";
  const showReviewBanner =
    hasJournalQuarantine || (activeTab !== "pricing" && hasPricingBlocker);
  const confLabel = confirmationsLabel(
    transaction.confirmations,
    t as (key: string, opts?: Record<string, unknown>) => string, // loose translator
  );
  const dirtyTags = dirty.tags;
  const dirtyLabel = dirty.label;
  const dirtyNote = dirty.note;
  const dirtyPricing = Boolean(
    dirty.pricingSourceKind ||
      dirty.pricingQuality ||
      dirty.manualCurrency ||
      dirty.manualPrice ||
      dirty.manualValue ||
      dirty.manualSource,
  );
  const dirtyExcluded = dirty.excluded;
  const dirtyReviewTax = Boolean(
    dirty.reviewStatus || dirty.taxable || dirty.atRegime || dirty.atCategory,
  );
  const graphData = graphQuery.data?.data;

  const timelineSteps: TimelineStep[] = [
    {
      key: "imported",
      label: t("sheet.timeline.imported"),
      done: true,
      hint: t("sheet.timeline.importedHint"),
    },
    {
      key: "settled",
      label: settlementLabel,
      done: transaction.status === "completed",
      current: transaction.status === "pending",
      hint:
        transaction.status === "completed"
          ? t("sheet.timeline.settledOnChain")
          : transaction.status === "pending"
            ? t("sheet.timeline.waitingConfirmation")
            : t("sheet.timeline.settlementIssue"),
    },
    {
      key: "reviewed",
      label:
        localDraft.reviewStatus === "review"
          ? t("sheet.timeline.needsReview")
          : t("sheet.timeline.reviewed"),
      done: localDraft.reviewStatus !== "review",
      hint: t("sheet.timeline.reviewedHint"),
    },
    {
      key: "journaled",
      label: localDraft.excluded
        ? t("sheet.timeline.excluded")
        : hasJournalQuarantine
          ? t("sheet.timeline.quarantined")
          : isPricingMissing
          ? t("sheet.timeline.pendingJournal")
          : t("sheet.timeline.journaled"),
      done: !localDraft.excluded && !hasJournalQuarantine && !isPricingMissing,
      hint: t("sheet.timeline.journaledHint"),
    },
  ];

  const reviewChecklistItems: Array<ChecklistItem & { tab?: string }> = [
    {
      key: "pricing",
      label: isPricingMissing
        ? t("sheet.checklist.setPricingSource")
        : t("sheet.checklist.pricedVia", {
            // dynamic key
            label: t(
              pricingSourceLabel(
                localDraft.pricingSourceKind,
                localDraft.pricingQuality,
              ) as ParseKeys<["transactions", "common"]>,
            ),
          }),
      done: !isPricingMissing,
      warn: isPricingMissing,
      tab: "pricing",
    },
    {
      key: "classified",
      label: isLabeled
        ? t("sheet.checklist.labeled", {
            label: classificationOptionLabelKeys[localDraft.label]
              ? // dynamic key
                t(
                  classificationOptionLabelKeys[
                    localDraft.label
                  ] as ParseKeys<["transactions", "common"]>,
                )
              : localDraft.label,
          })
        : t("sheet.checklist.pickLabel"),
      done: isLabeled,
      tab: "classify",
    },
    {
      key: "tax",
      label: localDraft.excluded
        ? t("sheet.checklist.excludedFromReports")
        : t("sheet.checklist.taxLabel", {
            // dynamic key
            label: t(
              taxClassification.shortLabel as ParseKeys<["transactions", "common"]>,
            ),
          }),
      done: true,
      tab: "tax",
    },
    {
      key: "quarantine",
      label: hasJournalQuarantine
        ? isSyncQuarantine
          ? t("sheet.checklist.resolveSyncMismatch")
          : isBasisQuarantine
            ? t("sheet.checklist.restoreBasis")
            : isTransferQuarantine
              ? t("sheet.checklist.reviewPairing")
              : t("sheet.checklist.resolveQuarantine")
        : hasPricingBlocker
          ? t("sheet.checklist.pricingIncomplete")
          : t("sheet.checklist.noQuarantine"),
      done: !hasJournalQuarantine && !hasPricingBlocker,
      warn: hasJournalQuarantine || hasPricingBlocker,
      tab: hasJournalQuarantine ? quarantineTargetTab : "pricing",
    },
  ];

  const addTag = (rawTag: string) => {
    const tag = rawTag.trim();
    if (!tag) return;
    updateDraft("tags", uniqueTags([...localDraft.tags, tag]));
    setTagInput("");
  };
  const removeTag = (tag: string) => {
    updateDraft(
      "tags",
      localDraft.tags.filter((candidate) => candidate !== tag),
    );
  };
  const availableTagSuggestions = tagSuggestions.filter(
    (suggestion) => !localDraft.tags.includes(suggestion),
  );
  const updateManualPrice = (rawPrice: string) => {
    const parsedPrice = parseManualDecimal(rawPrice);
    setLocalDraft((current) =>
      current
        ? {
            ...current,
            pricingSourceKind: "manual_override",
            pricingQuality: "exact",
            manualPrice: rawPrice,
            manualValue:
              parsedPrice !== null && amountBtc > 0
                ? formatManualFiat(parsedPrice * amountBtc)
                : "",
          }
        : current,
    );
  };
  const updateManualValue = (rawValue: string) => {
    const parsedValue = parseManualDecimal(rawValue);
    setLocalDraft((current) =>
      current
        ? {
            ...current,
            pricingSourceKind: "manual_override",
            pricingQuality: "exact",
            manualValue: rawValue,
            manualPrice:
              parsedValue !== null && amountBtc > 0
                ? formatManualPrice(parsedValue / amountBtc)
                : "",
          }
        : current,
    );
  };

  const jumpToManualPrice = () => {
    setActiveTab("pricing");
    setTimeout(() => manualPriceRef.current?.focus(), 0);
  };
  const jumpToQuarantineTarget = () => {
    if (hasJournalQuarantine && quarantineTargetTab !== "pricing") {
      setActiveTab(quarantineTargetTab);
      return;
    }
    jumpToManualPrice();
  };
  const chooseExactManualPrice = () => {
    updateDraft("pricingSourceKind", "manual_override");
    updateDraft("pricingQuality", "exact");
    jumpToManualPrice();
  };
  const openMarketDataSettings = () => {
    onOpenMarketDataSettings?.();
  };
  const setExcluded = () => updateDraft("excluded", true);
  const normalizedQuarantineReason = quarantineReason
    ? quarantineReason.replace(/_/g, " ")
    : null;
  const reviewBanner = showReviewBanner
    ? (() => {
        if (hasJournalQuarantine) {
          return {
            title: isBasisQuarantine
              ? t("tax.basisBlockerTitle")
              : t("sheet.banner.journalQuarantine"),
            reason: isBasisQuarantine
              ? t("tax.basisBlockerBody", {
                  asset: transaction.asset ?? "asset",
                })
              : t("sheet.banner.journalBlocker", {
                  reason: normalizedQuarantineReason,
                }),
            primaryActionLabel: isSyncQuarantine
              ? t("sheet.banner.viewDetails")
              : isBasisQuarantine
                ? t("sheet.banner.viewBasisContext")
                : isTransferQuarantine
                  ? t("sheet.banner.viewLinked")
                  : t("sheet.banner.viewPricing"),
          };
        }
        if (transaction.amount === null) {
          return {
            title: t("sheet.banner.missingFiatPrice"),
            reason: t("sheet.banner.noFiatRecorded", {
              date: transaction.date,
            }),
            hint: t("sheet.banner.readinessHint"),
            primaryActionLabel: t("sheet.banner.openPricing"),
          };
        }
        if (localDraft.pricingSourceKind === null) {
          return {
            title: t("sheet.banner.noPricingSource"),
            reason: t("sheet.banner.noPersistedSource"),
            hint: t("sheet.banner.readinessHint"),
            primaryActionLabel: t("sheet.banner.openPricing"),
          };
        }
        return {
          title: t("sheet.banner.pricingFlagged"),
          reason: t("sheet.banner.missingOrUnderReview"),
          hint: t("sheet.banner.readinessHint"),
          primaryActionLabel: t("sheet.banner.openPricing"),
        };
      })()
    : null;

  const taxNarrative = (() => {
    const action =
      flow === "incoming"
        ? t("tax.narrative.received")
        : flow === "outgoing"
          ? t("tax.narrative.sent")
          : t("tax.narrative.moved");
    const counterparty =
      transaction.counterparty || t("tax.narrative.theCounterparty");
    const at = transaction.date;
    const fiat = valueAtTimeEur
      ? t("tax.narrative.worthAtTime", {
          value: currencyFormatter.format(valueAtTimeEur),
        })
      : t("tax.narrative.noFiatYet");
    const treatment = localDraft.excluded
      ? t("tax.narrative.excludedTreatment")
      : t("tax.narrative.currentTreatment", {
          // dynamic key
          treatment: t(
            taxClassification.label as ParseKeys<["transactions", "common"]>,
          ),
        });
    return t("tax.narrative.sentence", {
      action,
      amount: formatBtcAmount(amountBtc),
      direction: flow === "outgoing" ? t("tax.narrative.to") : t("tax.narrative.from"),
      counterparty,
      date: at,
      fiat,
      treatment,
    });
  })();

  const tabContext: TransactionDetailTabContext = {
    transaction,
    localDraft,
    dirty,
    dirtyLabel,
    dirtyTags,
    dirtyNote,
    dirtyPricing,
    dirtyExcluded,
    dirtyReviewTax,
    hideSensitive,
    currency,
    transactionDisplayId,
    feeBtc,
    commercialContext,
    commercialContextLoading,
    showSourceExternalId,
    updateDraft,
    tags,
    tagInput,
    setTagInput,
    tagInputRef,
    addTag,
    removeTag,
    availableTagSuggestions,
    amountBtc,
    pricingValue,
    updateManualPrice,
    updateManualValue,
    manualPriceRef,
    hasCacheProvenance,
    isCoarsePricing,
    isProviderSamplePricing,
    isExactPricing,
    isPricingMissing,
    isBasisQuarantine,
    suppressPricingCacheWarning,
    suppressBasisQuarantineWarning,
    pricePoint,
    nowRate,
    onOpenMarketDataSettings,
    openMarketDataSettings,
    chooseExactManualPrice,
    flow,
    taxNarrative,
    taxClassification,
    valueAtTimeEur,
    pair,
    loanMark,
    linkedLoanMarks: linkedLoanMarks ?? [],
    loanLinkCandidates: loanLinkCandidates ?? [],
    onUnpair,
    isUnpairing,
    onOpenPairingReview,
    onLinkLoan,
    isLoanLinking,
    journalEvents,
    balanceCurrency,
    setBalanceCurrency,
    impactDirection,
    principalImpactBtc,
    principalImpactEur,
    feeImpactBtc,
    feeImpactEur,
    netImpactBtc,
    netImpactEur,
    graphData,
    graphLoading: graphQuery.isLoading || (graphQuery.isFetching && !graphData),
    graphError:
      graphQuery.error instanceof Error
        ? graphQuery.error.message
        : null,
  };

  return (
    <TooltipProvider delayDuration={150}>
      <Sheet open={Boolean(transaction)} onOpenChange={onOpenChange}>
        <SheetContent
          className="w-[min(100vw,1120px)] overflow-hidden p-0 sm:max-w-none"
          showCloseButton={false}
        >
          <TransactionDetailHeader
            transaction={transaction}
            flow={flow}
            reviewStatus={localDraft.reviewStatus}
            pair={pair}
            signedPrefix={signedPrefix}
            hideSensitive={hideSensitive}
            amountBtc={amountBtc}
            valueAtTimeEur={valueAtTimeEur}
            valueNowEur={valueNowEur}
            pricedChange={pricedChange}
            confLabel={confLabel}
            timelineSteps={timelineSteps}
            explorer={explorer}
            onOpenExplorer={onOpenExplorer}
            onClose={() => onOpenChange(false)}
          />

          <div className="min-h-0 flex-1 overflow-y-auto">
            <div className="grid gap-4 p-4 sm:p-6 xl:grid-cols-[minmax(0,1fr)_320px]">
              <div className="min-w-0 space-y-4">
                {reviewBanner ? (
                  <QuarantineBanner
                    title={reviewBanner.title}
                    reason={reviewBanner.reason}
                    hint={reviewBanner.hint}
                    primaryActionLabel={reviewBanner.primaryActionLabel}
                    onPrimaryAction={
                      hasJournalQuarantine && activeTab === quarantineTargetTab
                        ? undefined
                        : jumpToQuarantineTarget
                    }
                    onExclude={setExcluded}
                  />
                ) : null}

                {isSplitTransferQuarantine ? (
                  <TransactionSplitPayoutCard
                    transactionId={transaction.id}
                    sourceAsset={transaction.asset ?? "BTC"}
                    outboundBtc={amountBtc}
                  />
                ) : null}

                <Tabs value={activeTab} onValueChange={setActiveTab}>
                  <TabsList className="grid w-full grid-cols-6">
                    <TabsTrigger value="details">{t("sheet.tab.details")}</TabsTrigger>
                    <TabsTrigger value="classify">
                      {t("sheet.tab.classify")}
                      {dirtyLabel || dirtyTags || dirtyNote || dirty.reviewStatus ? (
                        <DirtyDot active />
                      ) : null}
                    </TabsTrigger>
                    <TabsTrigger value="pricing">
                      {t("sheet.tab.pricing")}
                      {dirtyPricing ? <DirtyDot active /> : null}
                    </TabsTrigger>
                    <TabsTrigger value="tax">
                      {t("sheet.tab.tax")}
                      {dirtyExcluded || dirtyReviewTax ? <DirtyDot active /> : null}
                    </TabsTrigger>
                    <TabsTrigger value="linked">{t("sheet.tab.linked")}</TabsTrigger>
                    <TabsTrigger value="ledger">{t("sheet.tab.ledger")}</TabsTrigger>
                  </TabsList>

                  <TransactionDetailsTab ctx={tabContext} />

                  <TransactionClassifyTab ctx={tabContext} />

                  <TransactionPricingTab ctx={tabContext} />

                  <TransactionTaxTab ctx={tabContext} />

                  <TransactionLinkedTab ctx={tabContext} />

                  <TransactionLedgerTab ctx={tabContext} />
                </Tabs>
              </div>

              <TransactionDetailRightRail
                transaction={transaction}
                sourceName={sourceName}
                sourceType={sourceType}
                explorer={explorer}
                reviewChecklistItems={reviewChecklistItems}
                onJumpTab={setActiveTab}
                hideSensitive={hideSensitive}
                attachments={attachments}
                onAddAttachmentFiles={onAddAttachmentFiles}
                onAddAttachmentLinks={onAddAttachmentLinks}
                onReuseEvidence={onReuseEvidence}
                onOpenAttachment={onOpenAttachment}
                onRenameAttachment={onRenameAttachment}
                onRemoveAttachment={onRemoveAttachment}
                tags={tags}
                dirtyTags={dirtyTags}
                historyEvents={historyEvents}
                historyStale={historyStale}
                historyLoading={historyLoading}
                isRevertingHistory={isRevertingHistory}
                onRevertHistory={onRevertHistory}
                onProcessJournals={onProcessJournals}
                isProcessingJournals={isProcessingJournals}
                onOpenExplorer={onOpenExplorer}
              />
            </div>
          </div>

          <SheetFooter className="border-t p-3 sm:flex-row sm:items-center sm:justify-between sm:px-6">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
              {dirtyCount > 0 ? (
                <span className="inline-flex items-center gap-1.5 font-medium text-amber-600 dark:text-amber-400">
                  <span className="inline-block size-1.5 rounded-full bg-amber-500" />
                  {t("sheet.footer.unsavedChanges", { count: dirtyCount })}
                </span>
              ) : null}
              <span className="hidden items-center gap-1.5 sm:inline-flex">
                <kbd className="rounded border bg-muted px-1">⌘S</kbd> {t("sheet.footer.shortcutSave")} ·{" "}
                <kbd className="rounded border bg-muted px-1">1-6</kbd> {t("sheet.footer.shortcutTabs")} ·{" "}
                <kbd className="rounded border bg-muted px-1">e</kbd> {t("sheet.footer.shortcutExclude")}
              </span>
              {saveError ? (
                <span className="basis-full text-destructive sm:basis-auto">
                  {saveError}
                </span>
              ) : null}
            </div>
            <div className="flex flex-wrap justify-end gap-2">
              {canMarkLoan && loanRole && onUnmarkLoan ? (
                <>
                  {flow === "outgoing" &&
                  loanRole !== "loan_principal_repaid" &&
                  onMarkLoan ? (
                    <Button
                      type="button"
                      variant="outline"
                      className="gap-1.5"
                      disabled={loanActionDisabled}
                      onClick={() => {
                        void onMarkLoan(transaction, "principal-repaid");
                      }}
                    >
                      <Coins className="size-4" aria-hidden="true" />
                      {t("table.row.collateral.changePrincipalRepaid")}
                    </Button>
                  ) : null}
                  {flow === "incoming" &&
                  loanRole !== "loan_principal_received" &&
                  onMarkLoan ? (
                    <Button
                      type="button"
                      variant="outline"
                      className="gap-1.5"
                      disabled={loanActionDisabled}
                      onClick={() => {
                        void onMarkLoan(transaction, "principal-received");
                      }}
                    >
                      <Coins className="size-4" aria-hidden="true" />
                      {t("table.row.collateral.changePrincipalReceived")}
                    </Button>
                  ) : null}
                  <Button
                    type="button"
                    variant="outline"
                    className="gap-1.5"
                    disabled={loanActionDisabled}
                    onClick={() => {
                      void onUnmarkLoan(transaction);
                    }}
                  >
                    <Link2Off className="size-4" aria-hidden="true" />
                    {t("table.row.collateral.unmark")}
                  </Button>
                </>
              ) : null}
              {canMarkLoan && !loanRole && flow === "incoming" && onMarkLoan ? (
                <>
                  <Button
                    type="button"
                    variant="outline"
                    className="gap-1.5"
                    disabled={loanActionDisabled}
                    onClick={() => {
                      void onMarkLoan(transaction, "principal-received");
                    }}
                  >
                    <Coins className="size-4" aria-hidden="true" />
                    {t("table.row.collateral.markPrincipalReceived")}
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    className="gap-1.5"
                    disabled={loanActionDisabled}
                    onClick={() => {
                      void onMarkLoan(transaction, "returned");
                    }}
                  >
                    <Coins className="size-4" aria-hidden="true" />
                    {t("table.row.collateral.markReturned")}
                  </Button>
                </>
              ) : null}
              {canMarkLoan && !loanRole && flow === "outgoing" && onMarkLoan ? (
                <>
                  <Button
                    type="button"
                    variant="outline"
                    className="gap-1.5"
                    disabled={loanActionDisabled}
                    onClick={() => {
                      void onMarkLoan(transaction, "principal-repaid");
                    }}
                  >
                    <Coins className="size-4" aria-hidden="true" />
                    {t("table.row.collateral.markPrincipalRepaid")}
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    className="gap-1.5"
                    disabled={loanActionDisabled}
                    onClick={() => {
                      void onMarkLoan(transaction, "collateral");
                    }}
                  >
                    <Coins className="size-4" aria-hidden="true" />
                    {t("table.row.collateral.markCollateral")}
                  </Button>
                </>
              ) : null}
              <Button
                type="button"
                variant="outline"
                disabled={isSaving}
                onClick={() => onOpenChange(false)}
              >
                {t("common:actions.cancel")}
              </Button>
              {dirtyCount > 0 ? (
                <Button
                  type="button"
                  variant="ghost"
                  className="gap-1.5 text-muted-foreground"
                  disabled={isSaving}
                  onClick={() => {
                    setLocalDraft(originalDraft);
                    setTagInput("");
                  }}
                >
                  <RotateCcw className="size-4" aria-hidden="true" />
                  {t("sheet.footer.discard")}
                </Button>
              ) : null}
              <Button
                type="button"
                className="gap-2"
                disabled={isSaving || dirtyCount === 0}
                onClick={async () => {
                  try {
                    if (onSaveAndNext && hasNext) {
                      await onSaveAndNext(transaction.id, localDraft);
                    } else {
                      await onSave(transaction.id, localDraft);
                      onOpenChange(false);
                    }
                  } catch {
                    // The parent renders the daemon error in the footer.
                  }
                }}
              >
                <Save className="size-4" aria-hidden="true" />
                {isSaving
                  ? t("sheet.footer.saving")
                  : onSaveAndNext && hasNext
                    ? t("sheet.footer.saveAndNext")
                    : t("sheet.footer.save")}
                {onSaveAndNext && hasNext && !isSaving ? (
                  <ArrowRight className="size-4" aria-hidden="true" />
                ) : null}
              </Button>
            </div>
          </SheetFooter>
        </SheetContent>
      </Sheet>
    </TooltipProvider>
  );
}
