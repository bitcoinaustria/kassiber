import * as React from "react";

import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { openAttachmentFile, openExternalUrl } from "@/daemon/transport";
import { type Currency } from "@/lib/currency";
import { type ExplorerSettings } from "@/lib/explorer";
import { useUiStore } from "@/store/ui";
import {
  ExplorerOpenDialog,
  TransactionDetailSheet,
  draftForTransaction,
  explorerForTransaction,
  parseManualDecimal,
  type CommercialContextData,
  type Transaction,
  type TransactionEditDraft,
} from "@/components/transactions";
import {
  attachmentRecordToItem,
  type AttachmentOpenData,
  type AttachmentRecord,
  type AttachmentsListData,
  type JournalEventsData,
  type SourceFundsLinksData,
} from "./model";

/**
 * Shared controller for the transaction detail sheet.
 *
 * Owns the supporting queries (attachments / source-funds links / journal /
 * commercial context), the metadata-save and attachment/unpair mutations,
 * draft state, and the explorer dialog. The parent owns *which* transaction is
 * open (controlled via `transaction`) plus any deep-link/URL handling, so this
 * one component backs both the Transactions screen and the Source-of-Funds
 * picker without duplicating the wiring.
 */
export function TransactionDetailController({
  transaction,
  initialTab = "details",
  hideSensitive,
  currency,
  explorerSettings,
  nowRate = null,
  navList = [],
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
  /** Called when the sheet requests to close. */
  onOpenChange: (open: boolean) => void;
  /** Called by "save & next" to advance to the next transaction. */
  onNavigate?: (txn: Transaction, tab: string) => void;
}) {
  // Local working copy so optimistic edits (e.g. unpair) can update the open
  // transaction even though `transaction` is controlled by the parent.
  const [workingTransaction, setWorkingTransaction] =
    React.useState<Transaction | null>(transaction);
  React.useEffect(() => {
    setWorkingTransaction(transaction);
  }, [transaction]);

  const [drafts, setDrafts] = React.useState<
    Record<string, TransactionEditDraft>
  >({});
  const [saveError, setSaveError] = React.useState<string | null>(null);
  const [explorerTransaction, setExplorerTransaction] =
    React.useState<Transaction | null>(null);

  const metadataUpdate = useDaemonMutation("ui.transactions.metadata.update");
  const attachmentAdd = useDaemonMutation<AttachmentRecord>("ui.attachments.add");
  const attachmentRemove = useDaemonMutation<AttachmentRecord>(
    "ui.attachments.remove",
  );
  const attachmentOpen =
    useDaemonMutation<AttachmentOpenData>("ui.attachments.open");
  const unpairTransfer = useDaemonMutation("ui.transfers.unpair");

  const detailId = transaction?.id ?? "";
  const enabled = Boolean(transaction);
  const attachmentsQuery = useDaemon<AttachmentsListData>(
    "ui.attachments.list",
    { transaction: detailId },
    { enabled },
  );
  const sourceFundsLinksQuery = useDaemon<SourceFundsLinksData>(
    "ui.source_funds.links.list",
    { target_transaction: detailId },
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

  const explorerTarget = explorerTransaction
    ? explorerForTransaction(explorerTransaction, explorerSettings)
    : null;

  const getDraft = React.useCallback(
    (txn: Transaction) => drafts[txn.id] ?? draftForTransaction(txn),
    [drafts],
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

  const attachmentItems = React.useMemo(
    () =>
      (attachmentsQuery.data?.data?.attachments ?? []).map(
        attachmentRecordToItem,
      ),
    [attachmentsQuery.data],
  );
  const sourceFundsLinks = sourceFundsLinksQuery.data?.data?.links ?? [];
  const journalEvents = journalEventsQuery.data?.data?.events ?? [];
  const commercialContext = commercialContextQuery.data?.data;

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
        sourceFundsLinks={sourceFundsLinks}
        journalEvents={journalEvents}
        commercialContext={commercialContext}
        commercialContextLoading={commercialContextQuery.isLoading}
        onAddAttachmentFiles={async (paths) => {
          if (!workingTransaction) return;
          for (const path of paths) {
            await attachmentAdd.mutateAsync({
              transaction: workingTransaction.id,
              file_path: path,
            });
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
          for (const url of urls) {
            await attachmentAdd.mutateAsync({
              transaction: workingTransaction.id,
              url,
            });
          }
          useUiStore.getState().addNotification({
            title: "Links attached",
            body: `${urls.length} link${urls.length === 1 ? "" : "s"} stored as attachment references.`,
            tone: "success",
            dedupeKey: `attachments-links-${workingTransaction.id}`,
          });
        }}
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
        onRemoveAttachment={async (item) => {
          await attachmentRemove.mutateAsync({ attachment: item.id });
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
        hasNext={
          workingTransaction
            ? navList.findIndex((txn) => txn.id === workingTransaction.id) <
              navList.length - 1
            : false
        }
        onOpenChange={(open) => {
          if (!open) {
            setSaveError(null);
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
    </>
  );
}
