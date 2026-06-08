import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { AddConnectionDialog } from "@/components/kb/AddConnectionDialog";
import type { RateLatestData } from "@/components/kb/settings/SettingsModel";
import { useDaemonMutation } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import { useCurrency } from "@/lib/currency";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { MOCK_OVERVIEW, type OverviewSnapshot } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

import { BooksHealthPanel } from "./BooksHealthPanel";
import { RecentTransactionsTable } from "./RecentTransactionsTable";
import { BtcActivityChart } from "./BtcActivityChart";
import { OverviewSidePanel } from "./OverviewSidePanel";
import { StatsCards } from "./StatsCards";
import {
  activeMarketFiatCurrency,
  activeMarketFiatRate,
  toDashboardTransaction as toOverviewTransaction,
  transactionRecords,
} from "./model";
import { useOverviewTransactionDetail } from "./useOverviewTransactionDetail";
import { WelcomeSection } from "./WelcomeSection";

export const OverviewDashboard = ({
  className,
  snapshot = MOCK_OVERVIEW,
  isSnapshotRefreshing = false,
}: {
  className?: string;
  snapshot?: OverviewSnapshot;
  isSnapshotRefreshing?: boolean;
}) => {
  const [addConnectionOpen, setAddConnectionOpen] = React.useState(false);
  const queryClient = useQueryClient();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const explorerSettings = useUiStore((s) => s.explorerSettings);
  const addNotification = useUiStore((s) => s.addNotification);
  const updateNotification = useUiStore((s) => s.updateNotification);
  const setActiveMaintenanceProgress = useUiStore(
    (s) => s.setActiveMaintenanceProgress,
  );
  const clearActiveMaintenanceProgress = useUiStore(
    (s) => s.clearActiveMaintenanceProgress,
  );
  const currency = useCurrency();
  const { syncAll, isSyncing } = useWalletSyncAction();
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction({ notifyStart: true });
  const refreshMarketRate = useDaemonMutation<RateLatestData>("ui.rates.latest");
  const marketRateNoticeRef = React.useRef<string | null>(null);
  const marketRateRefreshInFlightRef = React.useRef(false);
  const { detailSheet, openTransactionDetail } = useOverviewTransactionDetail({
    snapshot,
    hideSensitive,
    currency,
    explorerSettings,
  });
  const transactions = React.useMemo(
    () =>
      snapshot.txs.length
        ? snapshot.txs.map(toOverviewTransaction)
        : transactionRecords,
    [snapshot.txs],
  );
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const fiatRate = activeMarketFiatRate(snapshot);
  const refreshOverviewState = React.useCallback(() => {
    if (isSyncing || isProcessingJournals) return;
    syncAll();
  }, [isProcessingJournals, isSyncing, syncAll]);
  const refreshMarketRateState = React.useCallback(() => {
    if (marketRateRefreshInFlightRef.current) return;
    marketRateRefreshInFlightRef.current = true;
    const pair = snapshot.marketRate?.pair ?? undefined;
    const startedAt = new Date().toISOString();
    setActiveMaintenanceProgress({
      id: "market-rate-refresh",
      title: "Checking market rates",
      body: pair ? `Fetching ${pair}.` : "Fetching BTC market rate.",
      tone: "warning",
      progress: { indeterminate: true, label: "Refreshing BTC price" },
      details: ["Repricing transaction values"],
      active: true,
      startedAt,
      updatedAt: startedAt,
    });
    marketRateNoticeRef.current = addNotification({
      title: "BTC price refresh started",
      body: pair
        ? `Fetching the latest ${pair} quote.`
        : "Fetching the latest BTC quote.",
      tone: "warning",
      dedupeKey: "market-rate-refresh",
      progress: { indeterminate: true, label: "Refreshing" },
    });
    refreshMarketRate.mutate(
      {
        ...(pair ? { pair } : {}),
      },
      {
        onSuccess: (envelope) => {
          const rows =
            envelope.data?.latest.reduce(
              (total, row) => total + Number(row.samples ?? 0),
              0,
            ) ?? 0;
          const latestPair = envelope.data?.marketRate?.pair ?? pair;
          const body = rows
            ? `${latestPair ?? "BTC rate"} updated from the latest market sample.`
            : "No new market sample was returned; cached rate kept.";
          const notification = {
            title: "BTC price refreshed",
            body,
            tone: "success",
            dedupeKey: "market-rate-refresh",
            progress: undefined,
          } as const;
          if (marketRateNoticeRef.current) {
            updateNotification(marketRateNoticeRef.current, notification);
            marketRateNoticeRef.current = null;
            clearActiveMaintenanceProgress("market-rate-refresh");
            return;
          }
          addNotification(notification);
          clearActiveMaintenanceProgress("market-rate-refresh");
        },
        onError: (error) => {
          const body =
            error instanceof Error ? error.message : "Could not refresh BTC price.";
          const notification = {
            title: "BTC price refresh failed",
            body,
            tone: "error",
            dedupeKey: "market-rate-refresh",
            progress: undefined,
          } as const;
          if (marketRateNoticeRef.current) {
            updateNotification(marketRateNoticeRef.current, notification);
            marketRateNoticeRef.current = null;
            clearActiveMaintenanceProgress("market-rate-refresh");
            return;
          }
          addNotification(notification);
          clearActiveMaintenanceProgress("market-rate-refresh");
        },
        onSettled: () => {
          marketRateRefreshInFlightRef.current = false;
          void queryClient.invalidateQueries({ queryKey: ["daemon"] });
        },
      },
    );
  }, [
    addNotification,
    clearActiveMaintenanceProgress,
    queryClient,
    refreshMarketRate,
    setActiveMaintenanceProgress,
    snapshot.marketRate?.pair,
    updateNotification,
  ]);
  const isRefreshingOverview = isSyncing || isProcessingJournals;
  const overviewBusy =
    isSnapshotRefreshing || isRefreshingOverview || refreshMarketRate.isPending;

  return (
    <>
      <div
        className={cn(screenShellClassName, "relative", className)}
        aria-busy={overviewBusy}
      >
        <WelcomeSection
          snapshot={snapshot}
          onRefresh={refreshOverviewState}
          onProcessJournals={runJournalProcessing}
          isRefreshing={isRefreshingOverview}
          isProcessingJournals={isProcessingJournals}
          onAddConnection={() => setAddConnectionOpen(true)}
        />
        <AddConnectionDialog
          open={addConnectionOpen}
          onOpenChange={setAddConnectionOpen}
        />
        <StatsCards
          snapshot={snapshot}
          hideSensitive={hideSensitive}
          currency={currency}
          isRefreshing={isSnapshotRefreshing || isRefreshingOverview}
          isMarketRateRefreshing={refreshMarketRate.isPending}
          onRefreshMarketRate={refreshMarketRateState}
        />
        <div className="grid grid-cols-1 items-start gap-3 2xl:grid-cols-[minmax(0,1fr)_380px]">
          <div className="grid min-w-0 gap-3">
            <BtcActivityChart
              snapshot={snapshot}
              hideSensitive={hideSensitive}
              currency={currency}
              onOpenTransactionDetail={openTransactionDetail}
            />
            <RecentTransactionsTable
              className="min-w-0"
              transactions={transactions}
              hideSensitive={hideSensitive}
              currency={currency}
              priceEur={fiatRate}
              fiatCurrency={fiatCurrency}
            />
          </div>
          <div className="grid min-w-0 gap-3">
            <OverviewSidePanel
              snapshot={snapshot}
              hideSensitive={hideSensitive}
              currency={currency}
            />
            <BooksHealthPanel
              snapshot={snapshot}
              onProcessJournals={runJournalProcessing}
              isProcessingJournals={isProcessingJournals}
            />
          </div>
        </div>
      </div>
      {detailSheet}
    </>
  );
};
