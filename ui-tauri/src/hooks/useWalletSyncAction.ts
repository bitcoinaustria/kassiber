import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { daemonMutationKey, useDaemonMutation } from "@/daemon/client";
import { useUiStore } from "@/store/ui";
import { useSyncProgressNotice } from "./useSyncProgressNotice";

interface SyncResult {
  wallet: string;
  status: "synced" | "skipped" | "error" | string;
}

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
      title: "Wallet sync started",
      body: "Kassiber is syncing all configured wallet sources.",
      tone: "warning",
    });
    startSyncNotice();
    syncWallets.mutate(
      { all: true },
      {
        onSuccess: (envelope) => {
          const results = envelope.data?.results ?? [];
          const synced = results.filter(
            (result) => result.status === "synced",
          ).length;
          const skipped = results.filter(
            (result) => result.status === "skipped",
          ).length;
          const errors = results.filter(
            (result) => result.status === "error",
          ).length;
          const body =
            [
              synced ? `${synced} synced` : null,
              skipped ? `${skipped} skipped` : null,
              errors ? `${errors} failed` : null,
            ]
              .filter(Boolean)
              .join(", ") || "No wallet changes returned.";
          addNotification({
            title: errors
              ? "Wallet sync finished with errors"
              : "Wallet sync finished",
            body,
            tone: errors ? "error" : "success",
          });
        },
        onError: (error) => {
          addNotification({
            title: "Wallet sync failed",
            body: error instanceof Error ? error.message : "Wallet sync failed",
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
