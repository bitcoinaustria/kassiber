import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import {
  ExplorerOpenDialog,
  TransactionDetailSheet,
  TransactionEvidenceReuseDialog,
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
  matchesTransactionDeepLink,
  readTransactionDetailParams,
  removeAttachmentRecord,
  replaceAttachmentRecord,
  updateTransactionDetailParams,
  upsertAttachmentRecords,
  type AttachmentOpenData,
  type AttachmentRecord,
  type AttachmentsCopyData,
  type AttachmentsListData,
  type JournalEventsData,
} from "@/components/transactions/dashboard/model";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  openAttachmentFile,
  openExternalUrl,
  type DaemonEnvelope,
} from "@/daemon/transport";
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
  extraTransactions?: OverviewSnapshot["txs"];
  hideSensitive: boolean;
  currency: Currency;
  explorerSettings: ExplorerSettings;
};

export function useOverviewTransactionDetail({
  snapshot,
  extraTransactions = [],
  hideSensitive,
  currency,
  explorerSettings,
}: OverviewTransactionDetailOptions) {
  const { t } = useTranslation("transactions");
  const [detailTransaction, setDetailTransaction] =
    React.useState<Transaction | null>(null);
  const [detailInitialTab, setDetailInitialTab] = React.useState("details");
  const [explorerTransaction, setExplorerTransaction] =
    React.useState<Transaction | null>(null);
  const [drafts, setDrafts] = React.useState<
    Record<string, TransactionEditDraft>
  >({});
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [attachmentListOverride, setAttachmentListOverride] = React.useState<{
    transactionId: string;
    attachments: AttachmentRecord[];
  } | null>(null);
  const [reuseDialogOpen, setReuseDialogOpen] = React.useState(false);
  const [reuseSourceTransactionId, setReuseSourceTransactionId] =
    React.useState("");
  const pendingDetailLinkRef = React.useRef(readTransactionDetailParams());
  const queryClient = useQueryClient();
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
  const revertHistory = useDaemonMutation("ui.transactions.history.revert");
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction({
      notifyStart: true,
      notifyAlreadyRunning: true,
    });
  const attachmentsQuery = useDaemon<AttachmentsListData>(
    "ui.attachments.list",
    { transaction: detailTransaction?.id ?? "" },
    { enabled: Boolean(detailTransaction) },
  );
  const historyQuery = useDaemon<TransactionHistoryList>(
    "ui.transactions.history",
    { transaction: detailTransaction?.id ?? "", limit: 25 },
    { enabled: Boolean(detailTransaction) },
  );
  const reuseSourceAttachmentsQuery = useDaemon<AttachmentsListData>(
    "ui.attachments.list",
    { transaction: reuseSourceTransactionId },
    { enabled: reuseDialogOpen && Boolean(reuseSourceTransactionId) },
  );
  const journalEventsQuery = useDaemon<JournalEventsData>(
    "ui.journals.events.list",
    { transaction: detailTransaction?.id ?? "", limit: 20 },
    { enabled: Boolean(detailTransaction) },
  );
  const commercialContextQuery = useDaemon<CommercialContextData>(
    "ui.transactions.commercial_context",
    { transaction: detailTransaction?.id ?? "" },
    { enabled: Boolean(detailTransaction) },
  );
  const transactions = React.useMemo(
    () => overviewDetailTransactions(snapshot, extraTransactions),
    [extraTransactions, snapshot],
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
  const detailAttachmentRecords = React.useMemo(
    () => {
      if (
        attachmentListOverride &&
        attachmentListOverride.transactionId === detailTransaction?.id
      ) {
        return attachmentListOverride.attachments;
      }
      return attachmentsQuery.data?.data?.attachments ?? [];
    },
    [
      attachmentListOverride,
      attachmentsQuery.data?.data?.attachments,
      detailTransaction?.id,
    ],
  );
  const attachmentItems = React.useMemo(
    () =>
      detailAttachmentRecords.map((record) =>
        attachmentRecordToItem(record, t as (key: string) => string),
      ),
    [detailAttachmentRecords, t],
  );
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
  const evidenceSourceTransactions = React.useMemo(
    () =>
      detailTransaction
        ? transactions.filter((txn) => txn.id !== detailTransaction.id)
        : [],
    [detailTransaction, transactions],
  );
  React.useEffect(() => {
    if (!reuseDialogOpen) return;
    if (
      reuseSourceTransactionId &&
      evidenceSourceTransactions.some(
        (transaction) => transaction.id === reuseSourceTransactionId,
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
        attachments={detailTransaction ? attachmentItems : undefined}
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
            title: "Files attached",
            body: `${paths.length} file${paths.length === 1 ? "" : "s"} copied into Kassiber attachments.`,
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
            title: "Links attached",
            body: `${urls.length} link${urls.length === 1 ? "" : "s"} stored as attachment references.`,
            tone: "success",
            dedupeKey: `attachments-links-${detailTransaction.id}`,
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
            title: "Link text updated",
            body: "Attachment link label saved.",
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
            title: "Attachment removed",
            body:
              item.kind === "file"
                ? "Attachment record and copied file removed."
                : "Attachment link removed.",
            tone: "success",
            dedupeKey: `attachment-remove-${item.id}`,
          });
        }}
        onOpenChange={(open) => {
          if (!open) {
            setDetailTransaction(null);
            setReuseDialogOpen(false);
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
      <TransactionEvidenceReuseDialog
        open={reuseDialogOpen}
        onOpenChange={setReuseDialogOpen}
        targetTransaction={detailTransaction}
        sourceTransactions={evidenceSourceTransactions}
        sourceTransactionId={reuseSourceTransactionId}
        onSourceTransactionIdChange={setReuseSourceTransactionId}
        sourceAttachments={reuseSourceAttachmentItems}
        isLoadingSourceAttachments={reuseSourceAttachmentsQuery.isLoading}
        isCopying={attachmentCopy.isPending}
        hideSensitive={hideSensitive}
        onCopy={async (attachmentIds) => {
          if (!detailTransaction || !reuseSourceTransactionId) return;
          const result = await attachmentCopy.mutateAsync({
            transaction: detailTransaction.id,
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
              detailTransaction.id,
              (attachments) =>
                upsertAttachmentRecords(attachments, copiedAttachments),
            );
          }
          setReuseDialogOpen(false);
          useUiStore.getState().addNotification({
            title: "Evidence reused",
            body: `${copied} evidence item${copied === 1 ? "" : "s"} copied to this transaction.`,
            tone: "success",
            dedupeKey: `attachments-copy-${detailTransaction.id}`,
          });
        }}
      />
    </>
  );

  return { detailSheet, openTransactionDetail };
}
