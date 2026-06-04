import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { AddConnectionDialog } from "@/components/kb/AddConnectionDialog";
import type { RateRebuildData } from "@/components/kb/settings/SettingsModel";
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
  const [marketRateRefreshedAt, setMarketRateRefreshedAt] = React.useState<
    string | null
  >(null);
  const queryClient = useQueryClient();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const explorerSettings = useUiStore((s) => s.explorerSettings);
  const addNotification = useUiStore((s) => s.addNotification);
  const updateNotification = useUiStore((s) => s.updateNotification);
  const currency = useCurrency();
  const { syncAll, isSyncing } = useWalletSyncAction();
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction({ notifyStart: true });
  const refreshMarketRate = useDaemonMutation<RateRebuildData>("ui.rates.rebuild");
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
  React.useEffect(() => {
    setMarketRateRefreshedAt(null);
  }, [snapshot.marketRate?.pair]);
  const displayedSnapshot = React.useMemo(() => {
    if (!marketRateRefreshedAt || !snapshot.marketRate) return snapshot;
    return {
      ...snapshot,
      marketRate: {
        ...snapshot.marketRate,
        timestamp: marketRateRefreshedAt,
        fetchedAt: marketRateRefreshedAt,
        source: snapshot.marketRate.source ?? "coinbase-exchange",
      },
    };
  }, [marketRateRefreshedAt, snapshot]);
  const fiatCurrency = activeMarketFiatCurrency(displayedSnapshot);
  const fiatRate = activeMarketFiatRate(displayedSnapshot);
  const refreshOverviewState = React.useCallback(() => {
    if (isSyncing || isProcessingJournals) return;
    syncAll();
  }, [isProcessingJournals, isSyncing, syncAll]);
  const refreshMarketRateState = React.useCallback(() => {
    if (marketRateRefreshInFlightRef.current) return;
    marketRateRefreshInFlightRef.current = true;
    setMarketRateRefreshedAt(new Date().toISOString());
    const pair = snapshot.marketRate?.pair ?? undefined;
    marketRateNoticeRef.current = addNotification({
      title: "BTC price refresh started",
      body: pair ? `Fetching ${pair}.` : "Fetching BTC market rate.",
      tone: "warning",
      dedupeKey: "market-rate-refresh",
      progress: { indeterminate: true, label: "Refreshing" },
    });
    refreshMarketRate.mutate(
      {
        source: "coinbase-exchange",
        reprice_transactions: true,
        ...(pair ? { pair } : {}),
      },
      {
        onSuccess: (envelope) => {
          setMarketRateRefreshedAt(new Date().toISOString());
          const rows =
            envelope.data?.sync.reduce(
              (total, row) => total + Number(row.samples ?? 0),
              0,
            ) ?? 0;
          const notification = {
            title: "BTC price refreshed",
            body: rows ? `${rows.toLocaleString()} rate rows fetched.` : "Market rate refreshed.",
            tone: "success",
            dedupeKey: "market-rate-refresh",
            progress: undefined,
          } as const;
          if (marketRateNoticeRef.current) {
            updateNotification(marketRateNoticeRef.current, notification);
            marketRateNoticeRef.current = null;
            return;
          }
          addNotification(notification);
        },
        onError: (error) => {
          setMarketRateRefreshedAt(null);
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
            return;
          }
          addNotification(notification);
        },
        onSettled: () => {
          marketRateRefreshInFlightRef.current = false;
          void queryClient.invalidateQueries({ queryKey: ["daemon"] });
        },
      },
    );
  }, [
    addNotification,
    queryClient,
    refreshMarketRate,
    snapshot.marketRate?.pair,
    updateNotification,
  ]);
  const isRefreshingOverview = isSyncing || isProcessingJournals;
  const showRefreshSkeleton = isRefreshingOverview || isSnapshotRefreshing;

  return (
    <>
      <div
        className={cn(screenShellClassName, "relative", className)}
        aria-busy={showRefreshSkeleton}
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
          snapshot={displayedSnapshot}
          hideSensitive={hideSensitive}
          currency={currency}
          isRefreshing={showRefreshSkeleton}
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
