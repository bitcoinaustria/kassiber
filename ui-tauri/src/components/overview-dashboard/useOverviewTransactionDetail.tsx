import * as React from "react";

import {
  ExplorerOpenDialog,
  TransactionDetailSheet,
  draftForTransaction,
  explorerForTransaction,
  parseManualDecimal,
  type Transaction,
  type TransactionEditDraft,
} from "@/components/transactions";
import {
  matchesTransactionDeepLink,
  readTransactionDetailParams,
  updateTransactionDetailParams,
} from "@/components/transactions/dashboard/model";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import type { Currency } from "@/lib/currency";
import type { ExplorerSettings } from "@/lib/explorer";
import type {
  HistoryRevertTarget,
  TransactionHistoryList,
} from "@/lib/transactionHistory";
import type { OverviewSnapshot } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";
import { activeMarketFiatRate } from "./model";
import { overviewDetailTransactions } from "./overviewTransactionDetailModel";

type OverviewTransactionDetailOptions = {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
  explorerSettings: ExplorerSettings;
};

export function useOverviewTransactionDetail({
  snapshot,
  hideSensitive,
  currency,
  explorerSettings,
}: OverviewTransactionDetailOptions) {
  const [detailTransaction, setDetailTransaction] =
    React.useState<Transaction | null>(null);
  const [detailInitialTab, setDetailInitialTab] = React.useState("details");
  const [explorerTransaction, setExplorerTransaction] =
    React.useState<Transaction | null>(null);
  const [drafts, setDrafts] = React.useState<
    Record<string, TransactionEditDraft>
  >({});
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const pendingDetailLinkRef = React.useRef(readTransactionDetailParams());
  const metadataUpdate = useDaemonMutation("ui.transactions.metadata.update");
  const revertHistory = useDaemonMutation("ui.transactions.history.revert");
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction({
      notifyStart: true,
      notifyAlreadyRunning: true,
    });
  const historyQuery = useDaemon<TransactionHistoryList>(
    "ui.transactions.history",
    { transaction: detailTransaction?.id ?? "", limit: 25 },
    { enabled: Boolean(detailTransaction) },
  );
  const transactions = React.useMemo(
    () => overviewDetailTransactions(snapshot),
    [snapshot],
  );
  const getDraft = React.useCallback(
    (txn: Transaction) => drafts[txn.id] ?? draftForTransaction(txn),
    [drafts],
  );
  const saveTransactionDraft = React.useCallback(
    async (transactionId: string, draft: TransactionEditDraft) => {
      setSaveError(null);
      const sourceTransaction = transactions.find(
        (txn) => txn.id === transactionId,
      );
      const baseline = sourceTransaction
        ? drafts[transactionId] ?? draftForTransaction(sourceTransaction)
        : null;
      const persistedTagCodes = new Set(
        (sourceTransaction?.tags ?? []).map((tag) => tag.toLowerCase()),
      );
      const shouldPersistLabel =
        draft.label &&
        draft.label !== "Unlabeled" &&
        (persistedTagCodes.has(draft.label.toLowerCase()) ||
          draft.label !== baseline?.label);
      const tags = [
        shouldPersistLabel ? draft.label : "",
        ...draft.tags,
      ].filter(Boolean);
      const pricingDirty = baseline
        ? draft.pricingSourceKind !== baseline.pricingSourceKind ||
          draft.pricingQuality !== baseline.pricingQuality ||
          draft.manualCurrency !== baseline.manualCurrency ||
          draft.manualPrice !== baseline.manualPrice ||
          draft.manualValue !== baseline.manualValue ||
          draft.manualSource !== baseline.manualSource
        : false;
      const reviewTaxDirty = baseline
        ? draft.reviewStatus !== baseline.reviewStatus ||
          draft.taxable !== baseline.taxable ||
          draft.atRegime !== baseline.atRegime ||
          draft.atCategory !== baseline.atCategory
        : false;
      const manualPrice = parseManualDecimal(draft.manualPrice);
      const manualValue = parseManualDecimal(draft.manualValue);
      await metadataUpdate.mutateAsync({
        transaction: transactionId,
        note: draft.note.trim() ? draft.note : null,
        tags: Array.from(new Set(tags)),
        excluded: draft.excluded,
        ...(reviewTaxDirty
          ? {
              review_status: draft.reviewStatus,
              taxable: draft.taxable,
              at_regime: draft.atRegime,
              at_category: draft.atCategory,
            }
          : {}),
        ...(pricingDirty
          ? {
              pricing_source_kind: draft.pricingSourceKind,
              pricing_quality: draft.pricingQuality,
              fiat_currency: draft.manualCurrency.trim().toUpperCase(),
              fiat_rate: manualPrice === null ? null : draft.manualPrice,
              fiat_value: manualValue === null ? null : draft.manualValue,
              pricing_external_ref: draft.manualSource.trim() || null,
            }
          : {}),
      });
      setDrafts((current) => ({
        ...current,
        [transactionId]: draft,
      }));
    },
    [drafts, metadataUpdate, transactions],
  );
  const openTransactionDetail = React.useCallback(
    (transactionId: string, tab = "details") => {
      const transaction = transactions.find((txn) =>
        matchesTransactionDeepLink(txn, transactionId),
      );
      if (!transaction) return;
      setSaveError(null);
      setDetailInitialTab(tab);
      setDetailTransaction(transaction);
      updateTransactionDetailParams(transaction.id, tab);
    },
    [transactions],
  );
  const explorerTarget = explorerTransaction
    ? explorerForTransaction(explorerTransaction, explorerSettings)
    : null;
  const fiatRate = activeMarketFiatRate(snapshot);
  const historyData = historyQuery.data?.data;
  const revertHistoryTarget = React.useCallback(
    async (target: HistoryRevertTarget) => {
      if (!detailTransaction) return;
      await revertHistory.mutateAsync({
        transaction: detailTransaction.id,
        event: target.event.id,
        ...(target.field ? { field: target.field.field } : {}),
        reason: target.field
          ? `Reverted ${target.field.label} from edit history`
          : "Reverted edit history event",
      });
      useUiStore.getState().addNotification({
        title: "Edit reverted",
        body: "Kassiber wrote a new edit history entry with the reverted value.",
        tone: "success",
        dedupeKey: `history-revert-${target.event.id}-${target.field?.field ?? "event"}`,
      });
    },
    [detailTransaction, revertHistory],
  );

  React.useEffect(() => {
    const pending = pendingDetailLinkRef.current;
    if (!pending.transactionId) return;
    const transaction = transactions.find((txn) =>
      matchesTransactionDeepLink(txn, pending.transactionId ?? ""),
    );
    if (!transaction) return;
    pendingDetailLinkRef.current = { transactionId: null, tab: "details" };
    openTransactionDetail(transaction.id, pending.tab);
  }, [openTransactionDetail, transactions]);

  const detailSheet = (
    <>
      <ExplorerOpenDialog
        transaction={explorerTransaction}
        target={explorerTarget}
        onTransactionChange={setExplorerTransaction}
      />
      <TransactionDetailSheet
        transaction={detailTransaction}
        draft={detailTransaction ? getDraft(detailTransaction) : null}
        initialTab={detailInitialTab}
        hideSensitive={hideSensitive}
        currency={currency}
        explorerSettings={explorerSettings}
        isSaving={metadataUpdate.isPending}
        saveError={saveError}
        nowRate={fiatRate}
        historyEvents={historyData?.events}
        historyStale={historyData?.stale}
        historyLoading={historyQuery.isLoading}
        isRevertingHistory={revertHistory.isPending}
        onRevertHistory={revertHistoryTarget}
        onProcessJournals={runJournalProcessing}
        isProcessingJournals={isProcessingJournals}
        onOpenChange={(open) => {
          if (!open) {
            setDetailTransaction(null);
            setSaveError(null);
            updateTransactionDetailParams(null);
          }
        }}
        onOpenExplorer={(transaction) => setExplorerTransaction(transaction)}
        onSave={async (transactionId, draft) => {
          try {
            await saveTransactionDraft(transactionId, draft);
          } catch (error) {
            setSaveError(
              error instanceof Error ? error.message : "Could not save metadata.",
            );
            throw error;
          }
        }}
      />
    </>
  );

  return { detailSheet, openTransactionDetail };
}
