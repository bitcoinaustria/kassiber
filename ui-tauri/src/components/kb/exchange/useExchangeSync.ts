import * as React from "react";
import { useTranslation } from "react-i18next";

import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  endConnectionSync,
  recordConnectionSynced,
  recordConnectionSyncFailed,
  startConnectionSync,
} from "@/lib/connectionNotifications";
import { activeBookScope } from "@/lib/notificationScope";

import {
  EXCHANGE_PROVIDER_NAMES,
  exchangeErrorInfo,
  refusedPermissions,
  type ExchangeConnectionStatus,
  type ExchangeErrorInfo,
  type ExchangeKind,
  type ExchangeSyncResult,
} from "./exchangeApiModel";

/**
 * Sync now for one exchange wallet: the only request that reads the
 * exchange after setup. Ordinary refresh, background freshness and the
 * assistant leave exchange wallets alone, so this is the wallet page's
 * refresh for them.
 */
export function useExchangeSync({
  walletId,
  label,
  exchange,
}: {
  walletId: string;
  label: string;
  exchange: ExchangeKind | null;
}) {
  const { t } = useTranslation("connections");
  const status = useDaemon<{ connections: ExchangeConnectionStatus[] }>(
    "ui.connections.exchange.status",
    { wallet: walletId },
    // A display read: local only, and an older backend answers it with an
    // error that retrying cannot fix.
    { enabled: exchange !== null, retry: false },
  );
  const mutation = useDaemonMutation<ExchangeSyncResult>("ui.connections.exchange.sync");
  const [lastError, setLastError] = React.useState<ExchangeErrorInfo | null>(null);
  const provider = exchange ? EXCHANGE_PROVIDER_NAMES[exchange] : "";

  const sync = React.useCallback(async () => {
    if (!exchange || mutation.isPending) return;
    setLastError(null);
    // The book this sync belongs to, fixed at the start.
    const ref = {
      book: activeBookScope(),
      connectionId: walletId,
      label,
    };
    startConnectionSync(ref);
    try {
      const envelope = await mutation.mutateAsync({ wallet: walletId });
      recordConnectionSynced(ref, { imported: envelope.data?.imported ?? 0 });
    } catch (error) {
      const info = exchangeErrorInfo(error);
      setLastError(info);
      // The hint and any permissions the key still holds, which the wallet
      // page shows inline; the center keeps them in memory only.
      const permissions = refusedPermissions(info);
      recordConnectionSyncFailed(
        ref,
        info.message ?? t("exchange.error.unknown", { provider }),
        [
          info.hint ? { message: info.hint } : null,
          permissions.length
            ? {
                message: t("exchange.error.permissions", {
                  list: permissions.join(", "),
                }),
              }
            : null,
        ].filter((detail): detail is { message: string } => detail !== null),
      );
    } finally {
      endConnectionSync(walletId);
      void status.refetch();
    }
  }, [exchange, label, mutation, provider, status, t, walletId]);

  const row = status.data?.data?.connections?.[0] ?? null;
  const statusError = status.error ? exchangeErrorInfo(status.error) : null;
  return {
    sync,
    isPending: mutation.isPending,
    row,
    statusError,
    lastError,
  };
}
