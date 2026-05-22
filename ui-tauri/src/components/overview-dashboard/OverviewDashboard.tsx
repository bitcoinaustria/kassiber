import * as React from "react";

import { AddConnectionDialog } from "@/components/kb/AddConnectionDialog";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import { useCurrency } from "@/lib/currency";
import { screenShellClassName } from "@/lib/screen-layout";
import { cn } from "@/lib/utils";
import { MOCK_OVERVIEW, type OverviewSnapshot } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";

import { BooksHealthPanel } from "./BooksHealthPanel";
import { RecentTransactionsTable } from "./RecentTransactionsTable";
import { RevenueFlowChart } from "./RevenueFlowChart";
import { SideChartsSection } from "./SideChartsSection";
import { StatsCards } from "./StatsCards";
import { WelcomeSection } from "./WelcomeSection";
import { toDashboardTransaction, transactionRecords } from "./shared";

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
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const currency = useCurrency();
  const { syncAll, isSyncing } = useWalletSyncAction();
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction({ notifyStart: true });
  const transactions = React.useMemo(
    () =>
      snapshot.txs.length
        ? snapshot.txs.map(toDashboardTransaction)
        : transactionRecords,
    [snapshot.txs],
  );
  const refreshOverviewState = React.useCallback(() => {
    if (isSyncing || isProcessingJournals) return;
    syncAll({ onTrustedSuccess: runJournalProcessing });
  }, [isProcessingJournals, isSyncing, runJournalProcessing, syncAll]);
  const isRefreshingOverview = isSyncing || isProcessingJournals;
  const showRefreshSkeleton = isRefreshingOverview || isSnapshotRefreshing;

  return (
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
        snapshot={snapshot}
        hideSensitive={hideSensitive}
        currency={currency}
        isRefreshing={showRefreshSkeleton}
      />
      <div className="grid grid-cols-1 items-start gap-3 2xl:grid-cols-[minmax(0,1fr)_380px]">
        <div className="grid min-w-0 gap-3">
          <RevenueFlowChart
            snapshot={snapshot}
            hideSensitive={hideSensitive}
            currency={currency}
          />
          <RecentTransactionsTable
            className="min-w-0"
            transactions={transactions}
            hideSensitive={hideSensitive}
            currency={currency}
            priceEur={snapshot.priceEur}
          />
        </div>
        <div className="grid min-w-0 gap-3">
          <SideChartsSection
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
  );
};
