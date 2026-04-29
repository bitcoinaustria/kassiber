import * as React from "react";

import { useDaemonMutation } from "@/daemon/client";
import { useUiStore } from "@/store/ui";

interface SyncResult {
  wallet: string;
  status: "synced" | "skipped" | "error" | string;
}

export function useWalletSyncAction() {
  const addNotification = useUiStore((state) => state.addNotification);
  const syncWallets =
    useDaemonMutation<{ results: SyncResult[] }>("ui.wallets.sync");

  const syncAll = React.useCallback(() => {
    addNotification({
      title: "Wallet sync started",
      body: "Kassiber is syncing all configured wallet sources.",
      tone: "warning",
    });
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
      },
    );
  }, [addNotification, syncWallets]);

  return {
    syncAll,
    isSyncing: syncWallets.isPending,
  };
}
