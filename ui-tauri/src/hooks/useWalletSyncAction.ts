import * as React from "react";
import { useIsMutating, useQueryClient } from "@tanstack/react-query";

import { daemonMutationKey, useDaemonStreamMutation } from "@/daemon/client";
import {
  summarizeSyncResults,
  syncResultsAreTrustedForReports,
  type SyncResult,
} from "@/lib/syncResults";
import {
  STARTING_SYNC_PROGRESS_VALUE,
  startingSyncProgress,
  syncProgressNotification,
  type WalletSyncProgress,
} from "@/lib/syncProgress";
import { useUiStore } from "@/store/ui";

type WalletSyncOptions = {
  onTrustedSuccess?: (results: SyncResult[]) => void;
};

export function useWalletSyncAction() {
  const queryClient = useQueryClient();
  const dataMode = useUiStore((state) => state.dataMode);
  const walletSyncMutationKey = React.useMemo(
    () => daemonMutationKey(dataMode, "ui.wallets.sync"),
    [dataMode],
  );
  const walletSyncsInFlight = useIsMutating({
    mutationKey: walletSyncMutationKey,
  });
  const addNotification = useUiStore((state) => state.addNotification);
  const updateNotification = useUiStore((state) => state.updateNotification);
  const noticeIdRef = React.useRef<string | null>(null);
  const progressValueRef = React.useRef(STARTING_SYNC_PROGRESS_VALUE);
  const syncWallets = useDaemonStreamMutation<
    { results: SyncResult[] },
    WalletSyncProgress
  >("ui.wallets.sync", {
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
      // Honor in-flight syncs from any other `useDaemonMutation("ui.wallets.sync")`
      // instance — this hook is mounted in dashboard, overview, and shell surfaces, so a
      // sync started from one surface should block a duplicate from another.
      const otherSyncInFlight =
        queryClient.isMutating({
          mutationKey: walletSyncMutationKey,
        }) > 0;
      if (syncWallets.isPending || otherSyncInFlight) return;
      progressValueRef.current = STARTING_SYNC_PROGRESS_VALUE;
      noticeIdRef.current = addNotification({
        title: "Connection refresh started",
        body: "Kassiber is scanning configured watch-only sources.",
        tone: "warning",
        dedupeKey: "wallet-sync",
        progress: startingSyncProgress(),
      });
      syncWallets.mutate(
        { all: true },
        {
          onSuccess: (envelope) => {
            const results = envelope.data?.results ?? [];
            const errors = results.filter(
              (result) => result.status === "error",
            ).length;
            const body = summarizeSyncResults(results);
            const shouldRunFollowup = syncResultsAreTrustedForReports(results);
            if (noticeIdRef.current) {
              updateNotification(noticeIdRef.current, {
                title: errors
                  ? "Connection refresh finished with errors"
                  : "Connection refresh finished",
                body,
                tone: errors ? "error" : "success",
                dedupeKey: "wallet-sync",
                progress: undefined,
              });
              noticeIdRef.current = null;
            } else {
              addNotification({
                title: errors
                  ? "Connection refresh finished with errors"
                  : "Connection refresh finished",
                body,
                tone: errors ? "error" : "success",
                dedupeKey: "wallet-sync",
              });
            }
            if (shouldRunFollowup) options?.onTrustedSuccess?.(results);
          },
          onError: (error) => {
            const body =
              error instanceof Error
                ? error.message
                : "Connection refresh failed";
            if (noticeIdRef.current) {
              updateNotification(noticeIdRef.current, {
                title: "Connection refresh failed",
                body,
                tone: "error",
                dedupeKey: "wallet-sync",
                progress: undefined,
              });
              noticeIdRef.current = null;
              return;
            }
            addNotification({
              title: "Connection refresh failed",
              body,
              tone: "error",
              dedupeKey: "wallet-sync",
            });
          },
          onSettled: () => {
            void queryClient.invalidateQueries({ queryKey: ["daemon"] });
          },
        },
      );
    },
    [
      addNotification,
      queryClient,
      syncWallets,
      updateNotification,
      walletSyncMutationKey,
    ],
  );

  return {
    syncAll,
    isSyncing: walletSyncsInFlight > 0,
  };
}
