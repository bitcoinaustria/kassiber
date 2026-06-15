import * as React from "react";
import { useIsMutating, useQueryClient } from "@tanstack/react-query";

import { daemonMutationKey, useDaemonStreamMutation } from "@/daemon/client";
import {
  freshnessRunNeedsAttention,
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
        title: options?.forceFull ? "Rescanning book" : "Refreshing book",
        body: options?.forceFull
          ? "Kassiber is rescanning configured sources and journals."
          : "Kassiber is refreshing configured sources and journals.",
        tone: "warning",
        progress: startingSyncProgress(),
        details: ["Configured sources queued", "Journals included"],
        active: true,
        startedAt,
        updatedAt: startedAt,
      });
      noticeIdRef.current = addNotification({
        title: options?.forceFull ? "Book rescan started" : "Book refresh started",
        body: options?.forceFull
          ? "Kassiber is rescanning configured sources and journals."
          : "Kassiber is refreshing configured sources and journals.",
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
            if (noticeIdRef.current) {
              updateNotification(noticeIdRef.current, {
                title: needsAttention
                  ? "Book refresh needs attention"
                  : "Book refresh finished",
                body,
                tone: needsAttention ? "warning" : "success",
                dedupeKey: "book-refresh",
                progress: undefined,
              });
              noticeIdRef.current = null;
            } else {
              addNotification({
                title: needsAttention
                  ? "Book refresh needs attention"
                  : "Book refresh finished",
                body,
                tone: needsAttention ? "warning" : "success",
                dedupeKey: "book-refresh",
              });
            }
            if (!needsAttention) {
              options?.onTrustedSuccess?.();
              // The book has completed a clean full run, so subsequent
              // refreshes are ordinary background syncs rather than a
              // first-time setup. A run that still needs attention (job
              // errors, blocking reports) stays in first-sync mode so a retry
              // keeps the setup card instead of demoting to the thin line.
              if (bookKey) markFirstSyncDone(bookKey);
            }
            clearActiveMaintenanceProgress(BOOK_REFRESH_PROGRESS_ID);
            startedAtRef.current = null;
          },
          onError: (error) => {
            const body =
              error instanceof Error
                ? error.message
                : "Book refresh failed";
            if (noticeIdRef.current) {
              updateNotification(noticeIdRef.current, {
                title: "Book refresh failed",
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
              title: "Book refresh failed",
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
      updateNotification,
      walletSyncMutationKey,
    ],
  );

  return {
    syncAll,
    isSyncing: freshnessRunsInFlight + walletSyncsInFlight > 0,
  };
}
