import * as React from "react";
import { useTranslation } from "react-i18next";

import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { useUiStore } from "@/store/ui";

import {
  EXCHANGE_PROVIDER_NAMES,
  exchangeErrorInfo,
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
  const addNotification = useUiStore((state) => state.addNotification);
  const updateNotification = useUiStore((state) => state.updateNotification);
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
    const noticeId = addNotification({
      title: t("exchange.sync.startedTitle", { label }),
      body: t("exchange.sync.startedBody", { provider }),
      tone: "warning",
      dedupeKey: "wallet-sync",
    });
    try {
      const envelope = await mutation.mutateAsync({ wallet: walletId });
      const result = envelope.data ?? {};
      updateNotification(noticeId, {
        title: t("exchange.sync.finishedTitle", { label }),
        body: t("exchange.sync.finishedBody", {
          count: result.imported ?? 0,
          fetched: result.fetched ?? 0,
        }),
        tone: "success",
      });
    } catch (error) {
      const info = exchangeErrorInfo(error);
      setLastError(info);
      updateNotification(noticeId, {
        title: t("exchange.sync.failedTitle", { label }),
        body: info.message ?? t("exchange.error.unknown", { provider }),
        tone: "error",
      });
    } finally {
      void status.refetch();
    }
  }, [
    addNotification,
    exchange,
    label,
    mutation,
    provider,
    status,
    t,
    updateNotification,
    walletId,
  ]);

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
