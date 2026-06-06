import * as React from "react";
import { useIsMutating, useQueryClient } from "@tanstack/react-query";

import { daemonMutationKey, useDaemonStreamMutation } from "@/daemon/client";
import {
  freshnessRunNeedsAttention,
  summarizeFreshnessRun,
  type FreshnessRunData,
} from "@/lib/syncResults";
import {
  STARTING_SYNC_PROGRESS_VALUE,
  startingSyncProgress,
  syncProgressNotification,
  type WalletSyncProgress,
} from "@/lib/syncProgress";
import { useUiStore } from "@/store/ui";

type WalletSyncOptions = {
  onTrustedSuccess?: () => void;
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
  const noticeIdRef = React.useRef<string | null>(null);
  const progressValueRef = React.useRef(STARTING_SYNC_PROGRESS_VALUE);
  const refreshBook = useDaemonStreamMutation<
    FreshnessRunData,
    WalletSyncProgress
  >("ui.freshness.run", {
    onProgress: (progress) => {
      const noticeId = noticeIdRef.current;
      if (!noticeId) return;
      const nextProgress = syncProgressNotification(
        progress,
        progressValueRef.current,
      );
      progressValueRef.current = nextProgress.value;
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
      noticeIdRef.current = addNotification({
        title: "Book refresh started",
        body: "Kassiber is refreshing sources, market rates, and journals.",
        tone: "warning",
        dedupeKey: "book-refresh",
        progress: startingSyncProgress(),
      });
      refreshBook.mutate(
        { all: true, rates: true, journals: true, run: true },
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
            if (!needsAttention) options?.onTrustedSuccess?.();
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
              return;
            }
            addNotification({
              title: "Book refresh failed",
              body,
              tone: "error",
              dedupeKey: "book-refresh",
            });
          },
        },
      );
    },
    [
      addNotification,
      freshnessRunMutationKey,
      queryClient,
      refreshBook,
      updateNotification,
      walletSyncMutationKey,
    ],
  );

  return {
    syncAll,
    isSyncing: freshnessRunsInFlight + walletSyncsInFlight > 0,
  };
}
