import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import { AddConnectionDialog } from "@/components/kb/AddConnectionDialog";
import { Button } from "@/components/ui/button";
import {
  ExplorerOpenDialog,
  TransactionDetailSheet,
  draftForTransaction,
  metadataUpdateArgs,
  explorerForTransaction,
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
import {
  pageDescriptionClassName,
  pageHeaderClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import type {
  HistoryRevertTarget,
  TransactionHistoryList,
} from "@/lib/transactionHistory";
import { cn } from "@/lib/utils";
import type { Tx } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

import {
  QuarantineAssumptions,
  QuarantineCausePanel,
} from "./QuarantineCausePanel";
import { QuarantineQueue } from "./QuarantineQueue";
import {
  detailContextFor,
  quarantineRowTarget,
  type QuarantineDetailContext,
  type QuarantineSheetTab,
} from "./explain";
import type { QuarantineItem, QuarantineScope, QuarantineSnapshot } from "./types";

interface QuarantineDashboardProps {
  /** The "needs you" page: drives the summary and the cause cards. */
  attention: QuarantineSnapshot;
  /** The listed scope's page; the attention page when that is what is listed. */
  list: QuarantineSnapshot | null;
  listLoading: boolean;
  scope: QuarantineScope;
  onScopeChange: (scope: QuarantineScope) => void;
  offset: number;
  pageSize: number;
  onOffsetChange: (offset: number) => void;
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

type ConnectionDialogState =
  | { mode: "connect" }
  | { mode: "import"; walletId: string | null }
  | null;

type DetailTarget = { transactionId: string | null; tab: string };

function readQuarantineDetailTarget(): DetailTarget {
  const target = readTransactionDetailParams();
  return { transactionId: target.transactionId, tab: target.tab };
}

export function QuarantineDashboard({
  attention,
  list,
  listLoading,
  scope,
  onScopeChange,
  offset,
  pageSize,
  onOffsetChange,
  isProcessingJournals,
  onProcessJournals,
}: QuarantineDashboardProps) {
  const { t } = useTranslation("journals");
  const { t: tTransactions } = useTranslation("transactions");
  const currency = useCurrency();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const explorerSettings = useUiStore((s) => s.explorerSettings);
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [detailTarget, setDetailTarget] = React.useState(readQuarantineDetailTarget);
  // A cause can point at a root on another page; keep the daemon's reading
  // that came with the click so the sheet does not fall back to the code.
  const [openedContext, setOpenedContext] = React.useState<{
    transactionId: string;
    context: QuarantineDetailContext;
  } | null>(null);
  // "Save & next" walks the list the transaction was opened from.
  const [detailQueue, setDetailQueue] = React.useState<string[]>([]);
  const [dialog, setDialog] = React.useState<ConnectionDialogState>(null);
  // The causes are where the owner acts; the full list is one click away.
  const [showQueue, setShowQueue] = React.useState(false);
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
  const metadataUpdate = useDaemonMutation("ui.transactions.metadata.update");
  const attachmentAdd = useDaemonMutation<AttachmentRecord>("ui.attachments.add");
  const attachmentRename =
    useDaemonMutation<AttachmentRecord>("ui.attachments.rename");
  const attachmentRemove = useDaemonMutation<AttachmentRecord>(
    "ui.attachments.remove",
  );
  const attachmentOpen =
    useDaemonMutation<AttachmentOpenData>("ui.attachments.open");
  const unpairTransfer = useDaemonMutation("ui.transfers.unpair");
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
  const { summary } = attention;
  const listItems = list?.items ?? [];
  const knownItems = React.useMemo<QuarantineItem[]>(
    () => [...attention.items, ...(list && list !== attention ? list.items : [])],
    [attention, list],
  );
  const counts: Record<QuarantineScope, number> = {
    attention: summary.attention_count ?? summary.count,
    waiting: summary.waiting_count ?? 0,
    all: summary.count,
  };
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
  const queueIndex = detailTarget.transactionId
    ? detailQueue.indexOf(detailTarget.transactionId)
    : -1;
  const hasNext = queueIndex >= 0 && queueIndex < detailQueue.length - 1;
  const detailContext = detailContextFor(
    detailTarget.transactionId,
    knownItems,
    openedContext,
  );

  const openDetail = React.useCallback(
    (
      transactionId: string,
      tab: QuarantineSheetTab,
      context: QuarantineDetailContext | null,
      queue: string[],
    ) => {
      setSaveError(null);
      setOpenedContext(context ? { transactionId, context } : null);
      setDetailQueue(queue);
      setDetailTarget({ transactionId, tab });
      updateTransactionDetailParams(transactionId, tab, null);
    },
    [],
  );

  const closeDetail = React.useCallback(() => {
    setDetailTarget({ transactionId: null, tab: "details" });
    setOpenedContext(null);
    setDetailQueue([]);
    setExplorerTransaction(null);
    setSaveError(null);
    updateTransactionDetailParams(null);
  }, []);

  React.useEffect(() => {
    if (!detailTarget.transactionId || !transactionQuery.isError) return;
    const message =
      transactionQuery.error instanceof Error
        ? transactionQuery.error.message
        : t("quarantine.detail.resolveError");
    useUiStore.getState().addNotification({
      title: t("quarantine.detail.resolveError"),
      body: message,
      tone: "error",
      dedupeKey: `quarantine-resolve-${detailTarget.transactionId}`,
    });
    setDetailTarget({ transactionId: null, tab: "details" });
    setDetailQueue([]);
    updateTransactionDetailParams(null);
  }, [
    detailTarget.transactionId,
    t,
    transactionQuery.error,
    transactionQuery.isError,
  ]);

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
      await metadataUpdate.mutateAsync(
        metadataUpdateArgs({
          transactionId,
          draft,
          baseline,
          sourceTags: sourceTransaction?.tags ?? [],
        }),
      );
      setDrafts((current) => ({ ...current, [transactionId]: draft }));
    },
    [detailTransaction, drafts, metadataUpdate],
  );

  const saveAndOpenNext = React.useCallback(
    async (transactionId: string, draft: TransactionEditDraft) => {
      await saveTransactionDraft(transactionId, draft);
      await queryClient.refetchQueries({
        queryKey: ["daemon"],
        predicate: (query) =>
          query.queryKey.some((part) => part === "ui.journals.quarantine"),
      });
      const index = detailQueue.indexOf(transactionId);
      const next = index >= 0 ? detailQueue[index + 1] : undefined;
      if (!next) {
        closeDetail();
        return;
      }
      const nextItem = knownItems.find((item) => item.transaction_id === next);
      const target = nextItem
        ? quarantineRowTarget(nextItem)
        : { tab: "details" as const, context: null };
      openDetail(next, target.tab, target.context, detailQueue);
    },
    [closeDetail, detailQueue, knownItems, openDetail, queryClient, saveTransactionDraft],
  );

  const unpair = async (pairId: string) => {
    await unpairTransfer.mutateAsync({ pair_id: pairId });
    useUiStore.getState().addNotification({
      title: tTransactions("notification.pairRemoved.title"),
      body: tTransactions("notification.pairRemoved.body"),
      tone: "success",
      dedupeKey: `transfer-unpair-${pairId}`,
    });
  };

  const openFromList = (
    transactionId: string,
    tab: QuarantineSheetTab,
    context: QuarantineDetailContext | null,
  ) => openDetail(transactionId, tab, context, listItems.map((item) => item.transaction_id));

  return (
    <div className={cn(screenShellClassName)}>
      <div className={pageHeaderClassName}>
        <p className={cn(pageDescriptionClassName, "self-center")}>
          {t("quarantine.page.description")}
        </p>
      </div>

      <QuarantineCausePanel
        snapshot={attention}
        isProcessingJournals={isProcessingJournals}
        onProcessJournals={onProcessJournals}
        hideSensitive={hideSensitive}
        onConnectWallet={() => setDialog({ mode: "connect" })}
        onImportHistory={(walletId) => setDialog({ mode: "import", walletId })}
        onShowWaiting={() => {
          setShowQueue(true);
          onScopeChange("waiting");
        }}
        onOpenTransaction={(transactionId, tab, context) =>
          openDetail(
            transactionId,
            tab,
            context ?? null,
            (attention.summary.groups ?? []).flatMap((group) => group.root_transaction_ids),
          )
        }
      />

      {summary.count && !showQueue ? (
        <div>
          <Button type="button" variant="ghost" size="sm" onClick={() => setShowQueue(true)}>
            {t("quarantine.queue.show", { count: summary.count })}
          </Button>
        </div>
      ) : summary.count ? (
        <QuarantineQueue
          items={listItems}
          scope={scope}
          counts={counts}
          offset={offset}
          pageSize={pageSize}
          total={list?.summary.scope_count ?? counts[scope]}
          loading={listLoading}
          hideSensitive={hideSensitive}
          onScopeChange={onScopeChange}
          onOffsetChange={onOffsetChange}
          onOpenTransaction={openFromList}
          onHide={() => setShowQueue(false)}
        />
      ) : (
        <p className="kb-surface p-(--kb-card-padding) text-sm text-muted-foreground">
          {t("quarantine.empty")}
        </p>
      )}

      <QuarantineAssumptions
        assumptions={summary.assumptions ?? null}
        hideSensitive={hideSensitive}
        onConnectWallet={() => setDialog({ mode: "connect" })}
        onOpenTransaction={(transactionId, tab) => openDetail(transactionId, tab, null, [])}
      />

      {dialog ? (
        <AddConnectionDialog
          open
          initialSourceId={dialog.mode === "connect" ? "descriptor" : null}
          initialTargetWalletId={dialog.mode === "import" ? dialog.walletId : undefined}
          onOpenChange={(open) => {
            if (!open) setDialog(null);
          }}
        />
      ) : null}
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
        quarantineContext={detailContext}
        quarantineReasonOverride={detailContext?.reason ?? null}
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
        onUnpair={unpair}
        isUnpairing={unpairTransfer.isPending}
        onOpenPairingReview={() => {
          const focus = detailTransaction?.id;
          const reviewReason = detailContext?.reason.toLowerCase() ?? "";
          const ownershipReview =
            reviewReason.includes("ownership_transfer") ||
            reviewReason.includes("owned_fanout_unresolved");
          closeDetail();
          void navigate({
            to: "/swaps",
            search: {
              focus,
              method: ownershipReview ? "ownership_graph" : undefined,
            },
          });
        }}
        onOpenMarketDataSettings={() => {
          closeDetail();
          void navigate({ to: "/settings", hash: "market" });
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
    </div>
  );
}
