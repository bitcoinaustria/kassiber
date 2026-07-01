import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
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
  type CommercialContextData,
} from "@/components/transactions";
import {
  attachmentRecordToItem,
  isAttachmentListQueryKeyForTransaction,
  readTransactionDetailParams,
  removeAttachmentRecord,
  replaceAttachmentRecord,
  toDashboardTransaction,
  updateTransactionDetailParams,
  upsertAttachmentRecords,
  type AttachmentOpenData,
  type AttachmentRecord,
  type AttachmentsListData,
  type JournalEventsData,
} from "@/components/transactions/dashboard/model";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  openAttachmentFile,
  openExternalUrl,
  type DaemonEnvelope,
} from "@/daemon/transport";
import { useCurrency } from "@/lib/currency";
import type {
  HistoryRevertTarget,
  TransactionHistoryList,
} from "@/lib/transactionHistory";
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

type TransactionDetailTarget = ReturnType<typeof readTransactionDetailParams> & {
  rowId: string | null;
};

function readQuarantineDetailTarget(): TransactionDetailTarget {
  return { ...readTransactionDetailParams(), rowId: null };
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
  const queryClient = useQueryClient();
  const [detailTarget, setDetailTarget] = React.useState(
    readQuarantineDetailTarget,
  );
  const [explorerTransaction, setExplorerTransaction] =
    React.useState<Transaction | null>(null);
  const [drafts, setDrafts] = React.useState<
    Record<string, TransactionEditDraft>
  >({});
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [resolvePlanOpen, setResolvePlanOpen] = React.useState(false);
  const [attachmentListOverride, setAttachmentListOverride] = React.useState<{
    transactionId: string;
    attachments: AttachmentRecord[];
  } | null>(null);
  const metadataUpdate = useDaemonMutation("ui.transactions.metadata.update");
  const attachmentAdd = useDaemonMutation<AttachmentRecord>("ui.attachments.add");
  const attachmentRename =
    useDaemonMutation<AttachmentRecord>("ui.attachments.rename");
  const attachmentRemove = useDaemonMutation<AttachmentRecord>(
    "ui.attachments.remove",
  );
  const attachmentOpen =
    useDaemonMutation<AttachmentOpenData>("ui.attachments.open");
  const revertHistory = useDaemonMutation("ui.transactions.history.revert");
  const overviewQuery = useDaemon<OverviewSnapshot>("ui.overview.snapshot");
  const transactionQuery = useDaemon<TransactionResolveEnvelope>(
    "ui.transactions.resolve",
    { query: detailTarget.transactionId ?? "" },
    { enabled: Boolean(detailTarget.transactionId) },
  );
  const attachmentsQuery = useDaemon<AttachmentsListData>(
    "ui.attachments.list",
    { transaction: detailTarget.transactionId ?? "" },
    { enabled: Boolean(detailTarget.transactionId) },
  );
  const historyQuery = useDaemon<TransactionHistoryList>(
    "ui.transactions.history",
    { transaction: detailTarget.transactionId ?? "", limit: 25 },
    { enabled: Boolean(detailTarget.transactionId) },
  );
  const journalEventsQuery = useDaemon<JournalEventsData>(
    "ui.journals.events.list",
    { transaction: detailTarget.transactionId ?? "", limit: 20 },
    { enabled: Boolean(detailTarget.transactionId) },
  );
  const commercialContextQuery = useDaemon<CommercialContextData>(
    "ui.transactions.commercial_context",
    { transaction: detailTarget.transactionId ?? "" },
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
  const detailAttachmentRecords = React.useMemo(() => {
    if (
      attachmentListOverride &&
      attachmentListOverride.transactionId === detailTransaction?.id
    ) {
      return attachmentListOverride.attachments;
    }
    return attachmentsQuery.data?.data?.attachments ?? [];
  }, [
    attachmentListOverride,
    attachmentsQuery.data?.data?.attachments,
    detailTransaction?.id,
  ]);
  const attachmentItems = React.useMemo(
    () =>
      detailAttachmentRecords.map((record) =>
        attachmentRecordToItem(
          record,
          tTransactions as (key: string) => string,
        ),
      ),
    [detailAttachmentRecords, tTransactions],
  );
  const journalEvents = journalEventsQuery.data?.data?.events ?? [];
  const commercialContext = commercialContextQuery.data?.data;
  const historyData = historyQuery.data?.data;
  const selectedRowIndex = React.useMemo(() => {
    if (!detailTarget.transactionId) return -1;
    if (detailTarget.rowId) {
      const rowIndex = orderedRows.findIndex((row) => row.id === detailTarget.rowId);
      if (rowIndex >= 0) return rowIndex;
    }
    return orderedRows.findIndex(
      (row) =>
        row.transactionAction?.transactionId === detailTarget.transactionId &&
        (row.transactionAction.tab ?? "details") === detailTarget.tab,
    );
  }, [
    detailTarget.rowId,
    detailTarget.tab,
    detailTarget.transactionId,
    orderedRows,
  ]);
  const hasNext =
    selectedRowIndex >= 0 && selectedRowIndex < orderedRows.length - 1;

  const openDetail = React.useCallback(
    (
      action: NonNullable<ReviewTableRow["transactionAction"]>,
      row?: ReviewTableRow,
    ) => {
      setSaveError(null);
      const tab = action.tab ?? "details";
      const matchingRow =
        row ??
        orderedRows.find(
          (candidate) =>
            candidate.transactionAction?.transactionId === action.transactionId &&
            (candidate.transactionAction.tab ?? "details") === tab,
        ) ??
        orderedRows.find(
          (candidate) =>
            candidate.transactionAction?.transactionId === action.transactionId,
        );
      setDetailTarget({
        transactionId: action.transactionId,
        tab,
        rowId: matchingRow?.id ?? null,
      });
      updateTransactionDetailParams(action.transactionId, tab);
    },
    [orderedRows],
  );

  const closeDetail = React.useCallback(() => {
    setDetailTarget({ transactionId: null, tab: "details", rowId: null });
    setExplorerTransaction(null);
    setSaveError(null);
    updateTransactionDetailParams(null);
  }, []);

  React.useEffect(() => {
    setAttachmentListOverride(null);
  }, [detailTransaction?.id]);

  const updateDetailAttachmentRecords = React.useCallback(
    (updater: (attachments: AttachmentRecord[]) => AttachmentRecord[]) => {
      if (!detailTransaction) return;
      setAttachmentListOverride((current) => {
        const currentAttachments =
          current?.transactionId === detailTransaction.id
            ? current.attachments
            : attachmentsQuery.data?.data?.attachments ?? [];
        return {
          transactionId: detailTransaction.id,
          attachments: updater(currentAttachments),
        };
      });
    },
    [attachmentsQuery.data?.data?.attachments, detailTransaction],
  );

  const updateAttachmentListQueryCache = React.useCallback(
    (
      transactionId: string,
      updater: (attachments: AttachmentRecord[]) => AttachmentRecord[],
    ) => {
      queryClient.setQueriesData<DaemonEnvelope<AttachmentsListData>>(
        {
          queryKey: ["daemon"],
          predicate: (query) =>
            isAttachmentListQueryKeyForTransaction(
              query.queryKey,
              transactionId,
            ),
        },
        (current) =>
          current?.data
            ? {
                ...current,
                data: {
                  ...current.data,
                  attachments: updater(current.data.attachments),
                },
              }
            : current,
      );
    },
    [queryClient],
  );

  const revertHistoryTarget = React.useCallback(
    async (target: HistoryRevertTarget) => {
      if (!detailTransaction) return;
      await revertHistory.mutateAsync({
        transaction: detailTransaction.id,
        event: target.event.id,
        ...(target.field ? { field: target.field.field } : {}),
        reason: target.field
          ? tTransactions("history.revertReasonField", {
              label: target.field.label,
            })
          : tTransactions("history.revertReasonEvent"),
      });
      useUiStore.getState().addNotification({
        title: tTransactions("notification.editReverted.title"),
        body: tTransactions("notification.editReverted.body"),
        tone: "success",
        dedupeKey: `history-revert-${target.event.id}-${target.field?.field ?? "event"}`,
      });
    },
    [detailTransaction, revertHistory, tTransactions],
  );

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
        openDetail(next.transactionAction, next);
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
        const primaryRow = step.primaryRowId
          ? orderedRows.find((row) => row.id === step.primaryRowId)
          : undefined;
        openDetail(step.primaryAction, primaryRow);
      }
    },
    [onProcessJournals, openDetail, orderedRows],
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
        attachments={detailTransaction ? attachmentItems : undefined}
        journalEvents={journalEvents}
        commercialContext={commercialContext}
        commercialContextLoading={commercialContextQuery.isLoading}
        historyEvents={historyData?.events}
        historyStale={historyData?.stale}
        historyLoading={historyQuery.isLoading}
        isRevertingHistory={revertHistory.isPending}
        onRevertHistory={revertHistoryTarget}
        onProcessJournals={onProcessJournals}
        isProcessingJournals={isProcessingJournals}
        hasNext={hasNext}
        onAddAttachmentFiles={async (paths) => {
          if (!detailTransaction) return;
          const added: AttachmentRecord[] = [];
          for (const path of paths) {
            const result = await attachmentAdd.mutateAsync({
              transaction: detailTransaction.id,
              file_path: path,
            });
            if (result.data) {
              added.push(result.data);
            }
          }
          if (added.length) {
            updateDetailAttachmentRecords((attachments) =>
              upsertAttachmentRecords(attachments, added),
            );
            updateAttachmentListQueryCache(
              detailTransaction.id,
              (attachments) => upsertAttachmentRecords(attachments, added),
            );
          }
          useUiStore.getState().addNotification({
            title: tTransactions("notification.filesAttached.title"),
            body: tTransactions("notification.filesAttached.body", {
              count: paths.length,
            }),
            tone: "success",
            dedupeKey: `attachments-files-${detailTransaction.id}`,
          });
        }}
        onAddAttachmentLinks={async (urls) => {
          if (!detailTransaction) return;
          const added: AttachmentRecord[] = [];
          for (const url of urls) {
            const result = await attachmentAdd.mutateAsync({
              transaction: detailTransaction.id,
              url,
            });
            if (result.data) {
              added.push(result.data);
            }
          }
          if (added.length) {
            updateDetailAttachmentRecords((attachments) =>
              upsertAttachmentRecords(attachments, added),
            );
            updateAttachmentListQueryCache(
              detailTransaction.id,
              (attachments) => upsertAttachmentRecords(attachments, added),
            );
          }
          useUiStore.getState().addNotification({
            title: tTransactions("notification.linksAttached.title"),
            body: tTransactions("notification.linksAttached.body", {
              count: urls.length,
            }),
            tone: "success",
            dedupeKey: `attachments-links-${detailTransaction.id}`,
          });
        }}
        onOpenAttachment={async (item) => {
          const result = await attachmentOpen.mutateAsync({
            attachment: item.id,
          });
          const data = result.data;
          if (!data) return;
          if (data.target_type === "url" && data.url) {
            await openExternalUrl(data.url);
            return;
          }
          if (data.target_type === "file" && data.path) {
            await openAttachmentFile(data.path);
          }
        }}
        onRenameAttachment={async (item, label) => {
          if (!detailTransaction) return;
          const result = await attachmentRename.mutateAsync({
            attachment: item.id,
            label,
          });
          const updated = result.data;
          if (updated) {
            updateDetailAttachmentRecords((attachments) =>
              replaceAttachmentRecord(attachments, updated),
            );
            updateAttachmentListQueryCache(
              detailTransaction.id,
              (attachments) => replaceAttachmentRecord(attachments, updated),
            );
          }
          useUiStore.getState().addNotification({
            title: tTransactions("notification.linkTextUpdated.title"),
            body: tTransactions("notification.linkTextUpdated.body"),
            tone: "success",
          });
        }}
        onRemoveAttachment={async (item) => {
          if (!detailTransaction) return;
          await attachmentRemove.mutateAsync({ attachment: item.id });
          updateDetailAttachmentRecords((attachments) =>
            removeAttachmentRecord(attachments, item.id),
          );
          updateAttachmentListQueryCache(
            detailTransaction.id,
            (attachments) => removeAttachmentRecord(attachments, item.id),
          );
          useUiStore.getState().addNotification({
            title: tTransactions("notification.attachmentRemoved.title"),
            body:
              item.kind === "file"
                ? tTransactions("notification.attachmentRemoved.fileBody")
                : tTransactions("notification.attachmentRemoved.linkBody"),
            tone: "success",
            dedupeKey: `attachment-remove-${item.id}`,
          });
        }}
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
