import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  openAttachmentFile,
  openExternalUrl,
  type DaemonEnvelope,
} from "@/daemon/transport";
import { type Currency } from "@/lib/currency";
import { type ExplorerSettings } from "@/lib/explorer";
import { useUiStore } from "@/store/ui";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import type {
  HistoryRevertTarget,
  TransactionHistoryList,
} from "@/lib/transactionHistory";
import {
  ExplorerOpenDialog,
  TransactionDetailSheet,
  TransactionEvidenceReuseDialog,
  draftForTransaction,
  explorerForTransaction,
  parseManualDecimal,
  type CommercialContextData,
  type Transaction,
  type TransactionEditDraft,
} from "@/components/transactions";
import {
  attachmentRecordToItem,
  isAttachmentListQueryKeyForTransaction,
  removeAttachmentRecord,
  replaceAttachmentRecord,
  upsertAttachmentRecords,
  type AttachmentOpenData,
  type AttachmentRecord,
  type AttachmentsCopyData,
  type AttachmentsListData,
  type JournalEventsData,
} from "./model";

/**
 * Shared controller for the transaction detail sheet.
 *
 * Owns the supporting queries (attachments / journal events / edit history /
 * commercial context), the metadata-save, attachment, history-revert and
 * unpair mutations, draft state, the evidence-reuse dialog, and the explorer
 * dialog. The parent owns *which* transaction is open (controlled via
 * `transaction`) plus any deep-link/URL handling, so this one component backs
 * both the Transactions screen and the Source-of-Funds picker without
 * duplicating the wiring.
 */
export function TransactionDetailController({
  transaction,
  initialTab = "details",
  hideSensitive,
  currency,
  explorerSettings,
  nowRate = null,
  navList = [],
  evidenceSourceList,
  onOpenChange,
  onNavigate,
}: {
  transaction: Transaction | null;
  initialTab?: string;
  hideSensitive: boolean;
  currency: Currency;
  explorerSettings: ExplorerSettings;
  nowRate?: number | null;
  /** Ordered list used for "save & next" and the has-next affordance. */
  navList?: Transaction[];
  /**
   * Candidate pool for the evidence-reuse dialog. Defaults to `navList`,
   * but parents with filtered nav lists should pass the full loaded list so
   * an active table filter cannot hide reuse sources.
   */
  evidenceSourceList?: Transaction[];
  /** Called when the sheet requests to close. */
  onOpenChange: (open: boolean) => void;
  /** Called by "save & next" to advance to the next transaction. */
  onNavigate?: (txn: Transaction, tab: string) => void;
}) {
  const { t } = useTranslation("transactions");
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  // Local working copy so optimistic edits (e.g. unpair) can update the open
  // transaction even though `transaction` is controlled by the parent.
  const [workingTransaction, setWorkingTransaction] =
    React.useState<Transaction | null>(transaction);
  React.useEffect(() => {
    setWorkingTransaction(transaction);
    setSaveError(null);
  }, [transaction]);

  const [drafts, setDrafts] = React.useState<
    Record<string, TransactionEditDraft>
  >({});
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [explorerTransaction, setExplorerTransaction] =
    React.useState<Transaction | null>(null);
  const [attachmentListOverride, setAttachmentListOverride] = React.useState<{
    transactionId: string;
    attachments: AttachmentRecord[];
  } | null>(null);
  const [reuseDialogOpen, setReuseDialogOpen] = React.useState(false);
  const [reuseSourceTransactionId, setReuseSourceTransactionId] =
    React.useState("");

  const metadataUpdate = useDaemonMutation("ui.transactions.metadata.update");
  const attachmentAdd = useDaemonMutation<AttachmentRecord>("ui.attachments.add");
  const attachmentCopy = useDaemonMutation<AttachmentsCopyData>(
    "ui.attachments.copy",
  );
  const attachmentRename =
    useDaemonMutation<AttachmentRecord>("ui.attachments.rename");
  const attachmentRemove = useDaemonMutation<AttachmentRecord>(
    "ui.attachments.remove",
  );
  const attachmentOpen =
    useDaemonMutation<AttachmentOpenData>("ui.attachments.open");
  const unpairTransfer = useDaemonMutation("ui.transfers.unpair");
  const revertHistory = useDaemonMutation("ui.transactions.history.revert");
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction({
      notifyStart: true,
      notifyAlreadyRunning: true,
    });

  const detailId = transaction?.id ?? "";
  const enabled = Boolean(transaction);
  const attachmentsQuery = useDaemon<AttachmentsListData>(
    "ui.attachments.list",
    { transaction: detailId },
    { enabled },
  );
  const historyQuery = useDaemon<TransactionHistoryList>(
    "ui.transactions.history",
    { transaction: detailId, limit: 25 },
    { enabled },
  );
  const journalEventsQuery = useDaemon<JournalEventsData>(
    "ui.journals.events.list",
    { transaction: detailId, limit: 20 },
    { enabled },
  );
  const commercialContextQuery = useDaemon<CommercialContextData>(
    "ui.transactions.commercial_context",
    { transaction: detailId },
    { enabled },
  );
  const reuseSourceAttachmentsQuery = useDaemon<AttachmentsListData>(
    "ui.attachments.list",
    { transaction: reuseSourceTransactionId },
    { enabled: reuseDialogOpen && Boolean(reuseSourceTransactionId) },
  );

  const explorerTarget = explorerTransaction
    ? explorerForTransaction(explorerTransaction, explorerSettings)
    : null;

  const getDraft = React.useCallback(
    (txn: Transaction) => drafts[txn.id] ?? draftForTransaction(txn),
    [drafts],
  );

  // Optimistic attachment-list state: mutations patch the list shown in the
  // open sheet (and the per-transaction query caches) locally instead of
  // waiting for the global daemon-query invalidation round trip.
  const detailAttachmentRecords = React.useMemo(() => {
    if (
      attachmentListOverride &&
      attachmentListOverride.transactionId === workingTransaction?.id
    ) {
      return attachmentListOverride.attachments;
    }
    return attachmentsQuery.data?.data?.attachments ?? [];
  }, [
    attachmentListOverride,
    attachmentsQuery.data?.data?.attachments,
    workingTransaction?.id,
  ]);
  const attachmentItems = React.useMemo(
    () =>
      detailAttachmentRecords.map((record) =>
        attachmentRecordToItem(record, t as (key: string) => string),
      ),
    [detailAttachmentRecords, t],
  );
  React.useEffect(() => {
    setAttachmentListOverride(null);
  }, [workingTransaction?.id]);
  const updateDetailAttachmentRecords = React.useCallback(
    (updater: (attachments: AttachmentRecord[]) => AttachmentRecord[]) => {
      if (!workingTransaction) return;
      setAttachmentListOverride((current) => {
        const currentAttachments =
          current?.transactionId === workingTransaction.id
            ? current.attachments
            : attachmentsQuery.data?.data?.attachments ?? [];
        return {
          transactionId: workingTransaction.id,
          attachments: updater(currentAttachments),
        };
      });
    },
    [attachmentsQuery.data?.data?.attachments, workingTransaction],
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

  const evidencePool = evidenceSourceList ?? navList;
  const evidenceSourceTransactions = React.useMemo(
    () =>
      workingTransaction
        ? evidencePool.filter((txn) => txn.id !== workingTransaction.id)
        : [],
    [evidencePool, workingTransaction],
  );
  React.useEffect(() => {
    if (!reuseDialogOpen) return;
    if (
      reuseSourceTransactionId &&
      evidenceSourceTransactions.some(
        (txn) => txn.id === reuseSourceTransactionId,
      )
    ) {
      return;
    }
    setReuseSourceTransactionId(evidenceSourceTransactions[0]?.id ?? "");
  }, [evidenceSourceTransactions, reuseDialogOpen, reuseSourceTransactionId]);
  const reuseSourceAttachmentItems = React.useMemo(
    () =>
      (reuseSourceAttachmentsQuery.data?.data?.attachments ?? []).map((record) =>
        attachmentRecordToItem(record, t as (key: string) => string),
      ),
    [reuseSourceAttachmentsQuery.data, t],
  );

  const journalEvents = journalEventsQuery.data?.data?.events ?? [];
  const commercialContext = commercialContextQuery.data?.data;
  const historyData = historyQuery.data?.data;

  const revertHistoryTarget = React.useCallback(
    async (target: HistoryRevertTarget) => {
      if (!workingTransaction) return;
      await revertHistory.mutateAsync({
        transaction: workingTransaction.id,
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
    [revertHistory, workingTransaction],
  );

  const saveTransactionDraft = React.useCallback(
    async (transactionId: string, draft: TransactionEditDraft) => {
      setSaveError(null);
      const sourceTransaction =
        navList.find((txn) => txn.id === transactionId) ??
        (workingTransaction?.id === transactionId ? workingTransaction : null);
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
      const tags = [shouldPersistLabel ? draft.label : "", ...draft.tags].filter(
        Boolean,
      );
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
    [drafts, metadataUpdate, navList, workingTransaction],
  );

  return (
    <>
      <ExplorerOpenDialog
        transaction={explorerTransaction}
        target={explorerTarget}
        onTransactionChange={setExplorerTransaction}
      />
      <TransactionDetailSheet
        transaction={workingTransaction}
        draft={workingTransaction ? getDraft(workingTransaction) : null}
        initialTab={initialTab}
        hideSensitive={hideSensitive}
        currency={currency}
        explorerSettings={explorerSettings}
        isSaving={metadataUpdate.isPending}
        saveError={saveError}
        nowRate={nowRate}
        attachments={workingTransaction ? attachmentItems : undefined}
        journalEvents={journalEvents}
        commercialContext={commercialContext}
        commercialContextLoading={commercialContextQuery.isLoading}
        historyEvents={historyData?.events}
        historyStale={historyData?.stale}
        historyLoading={historyQuery.isLoading}
        isRevertingHistory={revertHistory.isPending}
        onRevertHistory={revertHistoryTarget}
        onProcessJournals={runJournalProcessing}
        isProcessingJournals={isProcessingJournals}
        onAddAttachmentFiles={async (paths) => {
          if (!workingTransaction) return;
          const added: AttachmentRecord[] = [];
          for (const path of paths) {
            const result = await attachmentAdd.mutateAsync({
              transaction: workingTransaction.id,
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
              workingTransaction.id,
              (attachments) => upsertAttachmentRecords(attachments, added),
            );
          }
          useUiStore.getState().addNotification({
            title: "Files attached",
            body: `${paths.length} file${paths.length === 1 ? "" : "s"} copied into Kassiber attachments.`,
            tone: "success",
            dedupeKey: `attachments-files-${workingTransaction.id}`,
          });
        }}
        onAddAttachmentLinks={async (urls) => {
          if (!workingTransaction) return;
          const added: AttachmentRecord[] = [];
          for (const url of urls) {
            const result = await attachmentAdd.mutateAsync({
              transaction: workingTransaction.id,
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
              workingTransaction.id,
              (attachments) => upsertAttachmentRecords(attachments, added),
            );
          }
          useUiStore.getState().addNotification({
            title: "Links attached",
            body: `${urls.length} link${urls.length === 1 ? "" : "s"} stored as attachment references.`,
            tone: "success",
            dedupeKey: `attachments-links-${workingTransaction.id}`,
          });
        }}
        onReuseEvidence={
          evidenceSourceTransactions.length
            ? () => {
                setReuseDialogOpen(true);
              }
            : undefined
        }
        onOpenAttachment={async (item) => {
          const result = await attachmentOpen.mutateAsync({ attachment: item.id });
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
          if (!workingTransaction) return;
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
              workingTransaction.id,
              (attachments) => replaceAttachmentRecord(attachments, updated),
            );
          }
          useUiStore.getState().addNotification({
            title: "Link text updated",
            body: "Attachment link label saved.",
            tone: "success",
          });
        }}
        onRemoveAttachment={async (item) => {
          if (!workingTransaction) return;
          await attachmentRemove.mutateAsync({ attachment: item.id });
          updateDetailAttachmentRecords((attachments) =>
            removeAttachmentRecord(attachments, item.id),
          );
          updateAttachmentListQueryCache(
            workingTransaction.id,
            (attachments) => removeAttachmentRecord(attachments, item.id),
          );
          useUiStore.getState().addNotification({
            title: "Attachment removed",
            body:
              item.kind === "file"
                ? "Attachment record and copied file removed."
                : "Attachment link removed.",
            tone: "success",
            dedupeKey: `attachment-remove-${item.id}`,
          });
        }}
        onUnpair={async (pairId) => {
          await unpairTransfer.mutateAsync({ pair_id: pairId });
          setWorkingTransaction((current) =>
            current?.pair?.id === pairId
              ? { ...current, pair: undefined }
              : current,
          );
          useUiStore.getState().addNotification({
            title: "Pair removed",
            body: "This movement is no longer linked to the other leg.",
            tone: "success",
            dedupeKey: `transfer-unpair-${pairId}`,
          });
        }}
        isUnpairing={unpairTransfer.isPending}
        onOpenMarketDataSettings={() => {
          setReuseDialogOpen(false);
          setSaveError(null);
          onOpenChange(false);
          void navigate({ to: "/settings", hash: "market" });
        }}
        hasNext={
          workingTransaction
            ? navList.findIndex((txn) => txn.id === workingTransaction.id) <
              navList.length - 1
            : false
        }
        onOpenChange={(open) => {
          if (!open) {
            setSaveError(null);
            setReuseDialogOpen(false);
            onOpenChange(false);
          }
        }}
        onOpenExplorer={(txn) => setExplorerTransaction(txn)}
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
        onSaveAndNext={async (transactionId, draft) => {
          try {
            await saveTransactionDraft(transactionId, draft);
            const idx = navList.findIndex((txn) => txn.id === transactionId);
            const next = navList[idx + 1];
            if (next && onNavigate) {
              onNavigate(next, initialTab);
            } else {
              onOpenChange(false);
            }
          } catch (error) {
            setSaveError(
              error instanceof Error ? error.message : "Could not save metadata.",
            );
            throw error;
          }
        }}
      />
      <TransactionEvidenceReuseDialog
        open={reuseDialogOpen}
        onOpenChange={setReuseDialogOpen}
        targetTransaction={workingTransaction}
        sourceTransactions={evidenceSourceTransactions}
        sourceTransactionId={reuseSourceTransactionId}
        onSourceTransactionIdChange={setReuseSourceTransactionId}
        sourceAttachments={reuseSourceAttachmentItems}
        isLoadingSourceAttachments={reuseSourceAttachmentsQuery.isLoading}
        isCopying={attachmentCopy.isPending}
        hideSensitive={hideSensitive}
        onCopy={async (attachmentIds) => {
          if (!workingTransaction || !reuseSourceTransactionId) return;
          const result = await attachmentCopy.mutateAsync({
            transaction: workingTransaction.id,
            source_transaction: reuseSourceTransactionId,
            attachments: attachmentIds,
          });
          const copied = result.data?.copied ?? attachmentIds.length;
          const copiedAttachments = result.data?.attachments ?? [];
          if (copiedAttachments.length) {
            updateDetailAttachmentRecords((attachments) =>
              upsertAttachmentRecords(attachments, copiedAttachments),
            );
            updateAttachmentListQueryCache(
              workingTransaction.id,
              (attachments) =>
                upsertAttachmentRecords(attachments, copiedAttachments),
            );
          }
          setReuseDialogOpen(false);
          useUiStore.getState().addNotification({
            title: "Evidence reused",
            body: `${copied} evidence item${copied === 1 ? "" : "s"} copied to this transaction.`,
            tone: "success",
            dedupeKey: `attachments-copy-${workingTransaction.id}`,
          });
        }}
      />
    </>
  );
}
