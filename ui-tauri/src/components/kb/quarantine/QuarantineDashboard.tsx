import * as React from "react";
import { useTranslation } from "react-i18next";

import {
  ReviewDataTable,
  type ReviewTableRow,
} from "@/components/kb/ReviewDataTable";
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
  readTransactionDetailParams,
  toDashboardTransaction,
  updateTransactionDetailParams,
} from "@/components/transactions/dashboard/model";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { useCurrency } from "@/lib/currency";
import type { Tx } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

import {
  quarantineMetrics,
  quarantineResolvePlan,
  quarantineRows,
  type QuarantineResolveStep,
} from "./model";
import { QuarantineActions } from "./QuarantineActions";
import { QuarantineResolveDrawer } from "./QuarantineResolveDrawer";
import type { QuarantineSnapshot } from "./types";

interface QuarantineDashboardProps {
  snapshot: QuarantineSnapshot;
  isProcessingJournals: boolean;
  onProcessJournals: () => void;
}

interface TransactionResolveEnvelope {
  transaction?: Tx | null;
  query?: string;
}

interface OverviewSnapshot {
  priceEur?: number | null;
}

export function QuarantineDashboard({
  snapshot,
  isProcessingJournals,
  onProcessJournals,
}: QuarantineDashboardProps) {
  const { t } = useTranslation("journals");
  const { t: tTransactions } = useTranslation("transactions");
  const currency = useCurrency();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const explorerSettings = useUiStore((s) => s.explorerSettings);
  const [detailTarget, setDetailTarget] = React.useState(
    readTransactionDetailParams,
  );
  const [explorerTransaction, setExplorerTransaction] =
    React.useState<Transaction | null>(null);
  const [drafts, setDrafts] = React.useState<
    Record<string, TransactionEditDraft>
  >({});
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [resolvePlanOpen, setResolvePlanOpen] = React.useState(false);
  const metadataUpdate = useDaemonMutation("ui.transactions.metadata.update");
  const overviewQuery = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const transactionQuery = useDaemon<TransactionResolveEnvelope>(
    "ui.transactions.resolve",
    { query: detailTarget.transactionId ?? "" },
    { enabled: Boolean(detailTarget.transactionId) },
  );
  const rows = React.useMemo(() => quarantineRows(snapshot, t), [snapshot, t]);
  const metrics = React.useMemo(
    () => quarantineMetrics(snapshot.summary, t),
    [snapshot.summary, t],
  );
  const resolvePlan = React.useMemo(
    () => quarantineResolvePlan(snapshot, rows, t),
    [rows, snapshot, t],
  );
  // Track the rows in the order the table actually shows them (search +
  // status/metric filters + sort), so "Save & next" advances through the
  // visible queue rather than the raw snapshot order.
  const [orderedRows, setOrderedRows] = React.useState<ReviewTableRow[]>(rows);
  const reasonGroupCount = snapshot.summary.by_reason.length;
  const detailTransaction = React.useMemo(() => {
    const tx = transactionQuery.data?.data?.transaction;
    return tx
      ? toDashboardTransaction(
          tx,
          0,
          tTransactions as (key: string, opts?: Record<string, unknown>) => string,
        )
      : null;
  }, [tTransactions, transactionQuery.data?.data?.transaction]);
  const explorerTarget = explorerTransaction
    ? explorerForTransaction(explorerTransaction, explorerSettings)
    : null;
  const selectedRowIndex = React.useMemo(
    () =>
      detailTarget.transactionId
        ? orderedRows.findIndex(
            (row) =>
              row.transactionAction?.transactionId === detailTarget.transactionId,
          )
        : -1,
    [detailTarget.transactionId, orderedRows],
  );
  const hasNext =
    selectedRowIndex >= 0 && selectedRowIndex < orderedRows.length - 1;

  const openDetail = React.useCallback(
    (
      action: NonNullable<ReviewTableRow["transactionAction"]>,
    ) => {
      setSaveError(null);
      const tab = action.tab ?? "details";
      setDetailTarget({ transactionId: action.transactionId, tab });
      updateTransactionDetailParams(action.transactionId, tab);
    },
    [],
  );

  const closeDetail = React.useCallback(() => {
    setDetailTarget({ transactionId: null, tab: "details" });
    setExplorerTransaction(null);
    setSaveError(null);
    updateTransactionDetailParams(null);
  }, []);

  const getDraft = React.useCallback(
    (txn: Transaction) => drafts[txn.id] ?? draftForTransaction(txn),
    [drafts],
  );

  const saveTransactionDraft = React.useCallback(
    async (transactionId: string, draft: TransactionEditDraft) => {
      setSaveError(null);
      const sourceTransaction =
        detailTransaction?.id === transactionId ? detailTransaction : null;
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
      setDrafts((current) => ({ ...current, [transactionId]: draft }));
    },
    [detailTransaction, drafts, metadataUpdate],
  );

  const saveAndOpenNext = React.useCallback(
    async (transactionId: string, draft: TransactionEditDraft) => {
      await saveTransactionDraft(transactionId, draft);
      const next = orderedRows[selectedRowIndex + 1];
      if (next?.transactionAction) {
        openDetail(next.transactionAction);
        return;
      }
      closeDetail();
    },
    [closeDetail, openDetail, orderedRows, saveTransactionDraft, selectedRowIndex],
  );

  const runResolveStep = React.useCallback(
    (step: QuarantineResolveStep) => {
      setResolvePlanOpen(false);
      if (step.actionKind === "process-journals") {
        onProcessJournals();
        return;
      }
      if (step.primaryAction) {
        openDetail(step.primaryAction);
      }
    },
    [onProcessJournals, openDetail],
  );

  return (
    <>
      <ReviewDataTable
        kind="quarantine"
        eyebrow={t("quarantine.eyebrow")}
        title={t("quarantine.title")}
        description={t("quarantine.description")}
        rows={rows}
        metrics={metrics}
        showSummaryBadge={false}
        badgeLabel={
          snapshot.summary.count
            ? t("quarantine.badge.quarantined", {
                count: snapshot.summary.count,
              })
            : t("quarantine.badge.clear")
        }
        tableTitle={t("quarantine.tableTitle")}
        tableDescription={t("quarantine.tableDescription", {
          count: reasonGroupCount,
          rows: rows.length,
        })}
        searchPlaceholder={t("quarantine.searchPlaceholder")}
        emptyMessage={t("quarantine.empty")}
        onOpenTransactionAction={openDetail}
        onVisibleRowsChange={setOrderedRows}
        actions={
          <QuarantineActions
            isProcessingJournals={isProcessingJournals}
            onProcessJournals={onProcessJournals}
            onOpenResolvePlan={() => setResolvePlanOpen(true)}
            resolvePlanCount={resolvePlan.total}
          />
        }
      />
      <QuarantineResolveDrawer
        open={resolvePlanOpen}
        plan={resolvePlan}
        isProcessingJournals={isProcessingJournals}
        onOpenChange={setResolvePlanOpen}
        onRunStep={runResolveStep}
      />
      <ExplorerOpenDialog
        transaction={explorerTransaction}
        target={explorerTarget}
        onTransactionChange={setExplorerTransaction}
      />
      <TransactionDetailSheet
        transaction={detailTransaction}
        draft={detailTransaction ? getDraft(detailTransaction) : null}
        initialTab={detailTarget.tab}
        hideSensitive={hideSensitive}
        currency={currency}
        explorerSettings={explorerSettings}
        isSaving={metadataUpdate.isPending}
        saveError={
          saveError ??
          (transactionQuery.isError && detailTarget.transactionId
            ? t("quarantine.detail.resolveError")
            : null)
        }
        nowRate={overviewQuery.data?.data?.priceEur ?? null}
        onProcessJournals={onProcessJournals}
        isProcessingJournals={isProcessingJournals}
        hasNext={hasNext}
        onOpenChange={(open) => {
          if (!open) closeDetail();
        }}
        onOpenExplorer={(transaction) => setExplorerTransaction(transaction)}
        onSave={async (transactionId, draft) => {
          try {
            await saveTransactionDraft(transactionId, draft);
            closeDetail();
          } catch (error) {
            setSaveError(
              error instanceof Error
                ? error.message
                : tTransactions("save.couldNotSaveMetadata"),
            );
            throw error;
          }
        }}
        onSaveAndNext={async (transactionId, draft) => {
          try {
            await saveAndOpenNext(transactionId, draft);
          } catch (error) {
            setSaveError(
              error instanceof Error
                ? error.message
                : tTransactions("save.couldNotSaveMetadata"),
            );
            throw error;
          }
        }}
      />
    </>
  );
}
