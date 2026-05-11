import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { daemonMutationKey, useDaemonMutation } from "@/daemon/client";
import { summarizeSyncResults, type SyncResult } from "@/lib/syncResults";
import { useUiStore } from "@/store/ui";
import { useSyncProgressNotice } from "./useSyncProgressNotice";

export function useWalletSyncAction() {
  const queryClient = useQueryClient();
  const dataMode = useUiStore((state) => state.dataMode);
  const addNotification = useUiStore((state) => state.addNotification);
  const syncWallets =
    useDaemonMutation<{ results: SyncResult[] }>("ui.wallets.sync");
  const { startSyncNotice, clearSyncNotice } = useSyncProgressNotice();

  const syncAll = React.useCallback(() => {
    // Honor in-flight syncs from any other `useDaemonMutation("ui.wallets.sync")`
    // instance — this hook is mounted in dashboard2/dashboard5/AppShell, so a
    // sync started from one surface should block a duplicate from another.
    const otherSyncInFlight =
      queryClient.isMutating({
        mutationKey: daemonMutationKey(dataMode, "ui.wallets.sync"),
      }) > 0;
    if (syncWallets.isPending || otherSyncInFlight) return;
    addNotification({
      title: "Connection refresh started",
      body: "Kassiber is scanning configured watch-only sources.",
      tone: "warning",
    });
    startSyncNotice();
    syncWallets.mutate(
      { all: true },
      {
        onSuccess: (envelope) => {
          const results = envelope.data?.results ?? [];
          const errors = results.filter(
            (result) => result.status === "error",
          ).length;
          const body = summarizeSyncResults(results);
          addNotification({
            title: errors
              ? "Connection refresh finished with errors"
              : "Connection refresh finished",
            body,
            tone: errors ? "error" : "success",
          });
        },
        onError: (error) => {
          addNotification({
            title: "Connection refresh failed",
            body: error instanceof Error ? error.message : "Connection refresh failed",
            tone: "error",
          });
        },
        onSettled: () => {
          clearSyncNotice();
          void queryClient.invalidateQueries({ queryKey: ["daemon"] });
        },
      },
    );
  }, [
    addNotification,
    clearSyncNotice,
    dataMode,
    queryClient,
    startSyncNotice,
    syncWallets,
  ]);

  return {
    syncAll,
    isSyncing: syncWallets.isPending,
  };
}
