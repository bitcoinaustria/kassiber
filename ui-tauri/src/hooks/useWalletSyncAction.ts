import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { daemonMutationKey, useDaemonStreamMutation } from "@/daemon/client";
import { summarizeSyncResults, type SyncResult } from "@/lib/syncResults";
import { useUiStore } from "@/store/ui";

type WalletSyncProgress = {
  phase?: string;
  wallet?: string;
  processed?: number;
  total?: number;
  imported?: number;
  skipped?: number;
};

function clampProgress(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}

function formatSyncProgressBody(progress: WalletSyncProgress) {
  const wallet = progress.wallet ? `${progress.wallet}: ` : "";
  const processed =
    typeof progress.processed === "number" ? progress.processed : null;
  const total = typeof progress.total === "number" ? progress.total : null;
  if (processed !== null && total !== null && total > 0) {
    return `${wallet}${processed.toLocaleString()} / ${total.toLocaleString()} rows scanned.`;
  }
  if (processed !== null) {
    return `${wallet}${processed.toLocaleString()} rows scanned.`;
  }
  return wallet
    ? `${wallet}refresh is running.`
    : "Kassiber is scanning configured watch-only sources.";
}

export function useWalletSyncAction() {
  const queryClient = useQueryClient();
  const dataMode = useUiStore((state) => state.dataMode);
  const addNotification = useUiStore((state) => state.addNotification);
  const updateNotification = useUiStore((state) => state.updateNotification);
  const noticeIdRef = React.useRef<string | null>(null);
  const syncWallets = useDaemonStreamMutation<
    { results: SyncResult[] },
    WalletSyncProgress
  >("ui.wallets.sync", {
    onProgress: (progress) => {
      const noticeId = noticeIdRef.current;
      if (!noticeId) return;
      const processed =
        typeof progress.processed === "number" ? progress.processed : null;
      const total = typeof progress.total === "number" ? progress.total : null;
      updateNotification(noticeId, {
        body: formatSyncProgressBody(progress),
        progress: {
          value:
            processed !== null && total !== null && total > 0
              ? clampProgress((processed / total) * 100)
              : undefined,
          indeterminate: !(processed !== null && total !== null && total > 0),
          label:
            processed !== null && total !== null && total > 0
              ? `${processed.toLocaleString()} / ${total.toLocaleString()}`
              : "Syncing",
        },
      });
    },
  });

  const syncAll = React.useCallback(() => {
    // Honor in-flight syncs from any other `useDaemonMutation("ui.wallets.sync")`
    // instance — this hook is mounted in dashboard2/dashboard5/AppShell, so a
    // sync started from one surface should block a duplicate from another.
    const otherSyncInFlight =
      queryClient.isMutating({
        mutationKey: daemonMutationKey(dataMode, "ui.wallets.sync"),
      }) > 0;
    if (syncWallets.isPending || otherSyncInFlight) return;
    noticeIdRef.current = addNotification({
      title: "Connection refresh started",
      body: "Kassiber is scanning configured watch-only sources.",
      tone: "warning",
      progress: {
        indeterminate: true,
        label: "Starting",
      },
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
          if (noticeIdRef.current) {
            updateNotification(noticeIdRef.current, {
              title: errors
                ? "Connection refresh finished with errors"
                : "Connection refresh finished",
              body,
              tone: errors ? "error" : "success",
              progress: undefined,
            });
            noticeIdRef.current = null;
            return;
          }
          addNotification({
            title: errors
              ? "Connection refresh finished with errors"
              : "Connection refresh finished",
            body,
            tone: errors ? "error" : "success",
          });
        },
        onError: (error) => {
          const body =
            error instanceof Error ? error.message : "Connection refresh failed";
          if (noticeIdRef.current) {
            updateNotification(noticeIdRef.current, {
              title: "Connection refresh failed",
              body,
              tone: "error",
              progress: undefined,
            });
            noticeIdRef.current = null;
            return;
          }
          addNotification({
            title: "Connection refresh failed",
            body,
            tone: "error",
          });
        },
        onSettled: () => {
          void queryClient.invalidateQueries({ queryKey: ["daemon"] });
        },
      },
    );
  }, [
    addNotification,
    dataMode,
    queryClient,
    syncWallets,
    updateNotification,
  ]);

  return {
    syncAll,
    isSyncing: syncWallets.isPending,
  };
}
