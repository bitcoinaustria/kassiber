import * as React from "react";
import { useIsMutating, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";

import { daemonMutationKey, useDaemonStreamMutation } from "@/daemon/client";
import {
  freshnessRunNeedsAttention,
  freshnessRunQuarantineCount,
  summarizeFreshnessRun,
  type FreshnessRunData,
} from "@/lib/syncResults";
import {
  activeSyncMaintenanceProgress,
  BOOK_REFRESH_PROGRESS_ID,
  STARTING_SYNC_PROGRESS_VALUE,
  startingSyncProgress,
  syncProgressNotification,
  type WalletSyncProgress,
} from "@/lib/syncProgress";
import { bookIdentityKey, useUiStore } from "@/store/ui";

type WalletSyncOptions = {
  onTrustedSuccess?: () => void;
  forceFull?: boolean;
};

export function useWalletSyncAction() {
  const { t } = useTranslation("overview");
  const queryClient = useQueryClient();
  const dataMode = useUiStore((state) => state.dataMode);
  const freshnessRunMutationKey = React.useMemo(
    () => daemonMutationKey(dataMode, "ui.freshness.run"),
    [dataMode],
  );
  const walletSyncMutationKey = React.useMemo(
    () => daemonMutationKey(dataMode, "ui.wallets.sync"),
    [dataMode],
  );
  const freshnessRunsInFlight = useIsMutating({
    mutationKey: freshnessRunMutationKey,
  });
  const walletSyncsInFlight = useIsMutating({ mutationKey: walletSyncMutationKey });
  const addNotification = useUiStore((state) => state.addNotification);
  const updateNotification = useUiStore((state) => state.updateNotification);
  const setActiveMaintenanceProgress = useUiStore(
    (state) => state.setActiveMaintenanceProgress,
  );
  const clearActiveMaintenanceProgress = useUiStore(
    (state) => state.clearActiveMaintenanceProgress,
  );
  const markFirstSyncDone = useUiStore((state) => state.markFirstSyncDone);
  const bookKey = useUiStore((state) => bookIdentityKey(state.identity));
  const noticeIdRef = React.useRef<string | null>(null);
  const startedAtRef = React.useRef<string | null>(null);
  const progressValueRef = React.useRef(STARTING_SYNC_PROGRESS_VALUE);
  const refreshBook = useDaemonStreamMutation<
    FreshnessRunData,
    WalletSyncProgress
  >("ui.freshness.run", {
    onProgress: (progress) => {
      const noticeId = noticeIdRef.current;
      if (!noticeId) return;
      const previousValue = progressValueRef.current;
      const nextProgress = syncProgressNotification(
        progress,
        previousValue,
      );
      progressValueRef.current = nextProgress.value;
      const now = new Date().toISOString();
      setActiveMaintenanceProgress(
        activeSyncMaintenanceProgress(progress, previousValue, {
          id: BOOK_REFRESH_PROGRESS_ID,
          startedAt: startedAtRef.current ?? now,
          updatedAt: now,
        }),
      );
      updateNotification(noticeId, {
        body: nextProgress.body,
        progress: nextProgress.progress,
      });
    },
  });

  const syncAll = React.useCallback(
    (options?: WalletSyncOptions) => {
      // Honor in-flight refreshes from any surface. This hook is mounted in the
      // transactions dashboard, overview dashboard, and AppShell, so one book
      // refresh should block duplicates while connection-detail syncs are also
      // running.
      const otherSyncInFlight =
        queryClient.isMutating({
          mutationKey: freshnessRunMutationKey,
        }) > 0 ||
        queryClient.isMutating({
          mutationKey: walletSyncMutationKey,
        }) > 0;
      if (refreshBook.isPending || otherSyncInFlight) return;
      progressValueRef.current = STARTING_SYNC_PROGRESS_VALUE;
      const startedAt = new Date().toISOString();
      startedAtRef.current = startedAt;
      setActiveMaintenanceProgress({
        id: BOOK_REFRESH_PROGRESS_ID,
        title: options?.forceFull
          ? t("bookRefresh.rescanInProgressTitle")
          : t("bookRefresh.refreshInProgressTitle"),
        body: options?.forceFull
          ? t("bookRefresh.rescanStartedBody")
          : t("bookRefresh.refreshStartedBody"),
        tone: "warning",
        progress: startingSyncProgress(),
        details: [
          t("bookRefresh.configuredSourcesQueued"),
          t("bookRefresh.journalsIncluded"),
        ],
        active: true,
        startedAt,
        updatedAt: startedAt,
      });
      noticeIdRef.current = addNotification({
        title: options?.forceFull
          ? t("bookRefresh.rescanStartedTitle")
          : t("bookRefresh.refreshStartedTitle"),
        body: options?.forceFull
          ? t("bookRefresh.rescanStartedBody")
          : t("bookRefresh.refreshStartedBody"),
        tone: "warning",
        dedupeKey: "book-refresh",
        progress: startingSyncProgress(),
      });
      refreshBook.mutate(
        {
          all: true,
          journals: true,
          run: true,
          force_full: Boolean(options?.forceFull),
        },
        {
          onSuccess: (envelope) => {
            const body = summarizeFreshnessRun(envelope.data);
            const needsAttention = freshnessRunNeedsAttention(envelope.data);
            const quarantineCount = freshnessRunQuarantineCount(envelope.data);
            const needsReview = needsAttention || quarantineCount > 0;
            const title = needsAttention
              ? t("bookRefresh.needsAttentionTitle")
              : quarantineCount > 0
                ? t("bookRefresh.quarantineTitle", { count: quarantineCount })
                : t("bookRefresh.finishedTitle");
            // Route the notification by a language-independent target instead of
            // letting the header guess from the (localized) title: failures /
            // blocking sources go to the logs (settings when dev tools are off),
            // quarantine goes to the quarantine review.
            const target = needsAttention
              ? "/logs"
              : quarantineCount > 0
                ? "/quarantine"
                : undefined;
            if (noticeIdRef.current) {
              updateNotification(noticeIdRef.current, {
                title,
                body,
                tone: needsReview ? "warning" : "success",
                dedupeKey: "book-refresh",
                progress: undefined,
                target,
              });
              noticeIdRef.current = null;
            } else {
              addNotification({
                title,
                body,
                tone: needsReview ? "warning" : "success",
                dedupeKey: "book-refresh",
                target,
              });
            }
            if (!needsReview) {
              options?.onTrustedSuccess?.();
              // The book has completed a clean full run, so subsequent
              // refreshes are ordinary background syncs rather than a
              // first-time setup. A run that still needs attention (job
              // errors, blocking reports, or journal quarantine) stays in
              // first-sync mode so a retry keeps the setup card instead of
              // demoting to the thin line.
              if (bookKey) markFirstSyncDone(bookKey);
            }
            clearActiveMaintenanceProgress(BOOK_REFRESH_PROGRESS_ID);
            startedAtRef.current = null;
          },
          onError: (error) => {
            const body =
              error instanceof Error
                ? error.message
                : t("bookRefresh.failedBody");
            if (noticeIdRef.current) {
              updateNotification(noticeIdRef.current, {
                title: t("bookRefresh.failedTitle"),
                body,
                tone: "error",
                dedupeKey: "book-refresh",
                progress: undefined,
              });
              noticeIdRef.current = null;
              clearActiveMaintenanceProgress(BOOK_REFRESH_PROGRESS_ID);
              startedAtRef.current = null;
              return;
            }
            addNotification({
              title: t("bookRefresh.failedTitle"),
              body,
              tone: "error",
              dedupeKey: "book-refresh",
            });
            clearActiveMaintenanceProgress(BOOK_REFRESH_PROGRESS_ID);
            startedAtRef.current = null;
          },
        },
      );
    },
    [
      addNotification,
      bookKey,
      clearActiveMaintenanceProgress,
      freshnessRunMutationKey,
      markFirstSyncDone,
      queryClient,
      refreshBook,
      setActiveMaintenanceProgress,
      t,
      updateNotification,
      walletSyncMutationKey,
    ],
  );

  return {
    syncAll,
    isSyncing: freshnessRunsInFlight + walletSyncsInFlight > 0,
  };
}
