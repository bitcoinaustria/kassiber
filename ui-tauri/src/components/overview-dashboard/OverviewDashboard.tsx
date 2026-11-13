import * as React from "react";
import { useQueryClient } from "@tanstack/react-query";

import { AddConnectionDialog } from "@/components/kb/AddConnectionDialog";
import type { RateLatestData } from "@/components/kb/settings/SettingsModel";
import { useDaemonMutation } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import i18n from "@/i18n";
import { useCurrency } from "@/lib/currency";
import { showFeedback } from "@/lib/feedback";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

import { RecentTransactionsTable } from "./RecentTransactionsTable";
import { BtcActivityChart } from "./BtcActivityChart";
import { HoldingsBySourceChart } from "./HoldingsBySource";
import { StatsCards } from "./StatsCards";
import {
  activeMarketFiatCurrency,
  activeMarketFiatRate,
  overviewTransactions,
} from "./model";
import { useOverviewTransactionDetail } from "./useOverviewTransactionDetail";
import { WelcomeSection } from "./WelcomeSection";

/** Transactions listed, fewer beside the chart if fewer fit; the rest are one click away. */
const RECENT_ROWS = 12;

export const OverviewDashboard = ({
  className,
  snapshot,
  isSnapshotRefreshing = false,
}: {
  className?: string;
  snapshot: OverviewSnapshot;
  isSnapshotRefreshing?: boolean;
}) => {
  const [addConnectionOpen, setAddConnectionOpen] = React.useState(false);
  const queryClient = useQueryClient();
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const explorerSettings = useUiStore((s) => s.explorerSettings);
  const setActiveMaintenanceProgress = useUiStore(
    (s) => s.setActiveMaintenanceProgress,
  );
  const clearActiveMaintenanceProgress = useUiStore(
    (s) => s.clearActiveMaintenanceProgress,
  );
  const currency = useCurrency();
  const { syncAll, isSyncing } = useWalletSyncAction();
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction();
  const refreshMarketRate = useDaemonMutation<RateLatestData>("ui.rates.latest");
    const marketRateRefreshInFlightRef = React.useRef(false);
  const { detailSheet, openTransactionDetail } = useOverviewTransactionDetail({
    snapshot,
    hideSensitive,
    currency,
    explorerSettings,
  });
  const transactions = React.useMemo(
    () => overviewTransactions(snapshot),
    [snapshot],
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
      state: "running",
      startedAt,
      updatedAt: startedAt,
    });
    // The shell's progress line shows it running; only the outcome is news.
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
          showFeedback({
            title: rows
              ? i18n.t("chrome:notifications.feedback.btcPriceRefreshed")
              : i18n.t("chrome:notifications.feedback.btcPriceUnchanged"),
            tone: rows ? "success" : "info",
            key: "market-rate-refresh",
          });
          clearActiveMaintenanceProgress("market-rate-refresh");
        },
        onError: (error) => {
          // The quote keeps its cached value; the error is free text.
          showFeedback({
            title: i18n.t("chrome:notifications.feedback.btcPriceFailed"),
            description: error instanceof Error ? error.message : undefined,
            descriptionPrivacy: "free-text",
            tone: "error",
            key: "market-rate-refresh",
          });
          clearActiveMaintenanceProgress("market-rate-refresh");
        },
        onSettled: () => {
          marketRateRefreshInFlightRef.current = false;
          void queryClient.invalidateQueries({ queryKey: ["daemon"] });
        },
      },
    );
  }, [
    clearActiveMaintenanceProgress,
    queryClient,
    refreshMarketRate,
    setActiveMaintenanceProgress,
    snapshot.marketRate?.pair,
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
          onProcessJournals={runJournalProcessing}
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
          isRefreshing={isSnapshotRefreshing || isRefreshingOverview}
          isMarketRateRefreshing={refreshMarketRate.isPending}
          onRefreshMarketRate={refreshMarketRateState}
        />
        {/* The latest transactions sit beside the chart where the page is wide
            enough, so both are in view without scrolling; below that they
            follow it. Either way the list never scrolls inside the page. */}
        <div className="@container/overview-main">
        <div className="grid grid-cols-1 gap-(--kb-page-gap) @min-[64rem]/overview-main:grid-cols-[minmax(0,1fr)_minmax(320px,30%)]">
          <BtcActivityChart
            snapshot={snapshot}
            hideSensitive={hideSensitive}
            currency={currency}
            onOpenTransactionDetail={openTransactionDetail}
            onRefresh={refreshOverviewState}
            onAddConnection={() => setAddConnectionOpen(true)}
            isRefreshing={isRefreshingOverview}
          />
          {/* Beside the chart the list takes the chart's height, not its own:
              rows that do not fit are left to Show all. */}
          <div className="relative min-w-0">
            <RecentTransactionsTable
              className="@min-[64rem]/overview-main:absolute @min-[64rem]/overview-main:inset-0"
              transactions={transactions}
              hideSensitive={hideSensitive}
              currency={currency}
              priceEur={fiatRate}
              fiatCurrency={fiatCurrency}
              limit={RECENT_ROWS}
            />
          </div>
        </div>
        </div>
        <HoldingsBySourceChart
          snapshot={snapshot}
          hideSensitive={hideSensitive}
          currency={currency}
        />
      </div>
      {detailSheet}
    </>
  );
};
