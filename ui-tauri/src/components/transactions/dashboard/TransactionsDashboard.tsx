import { RefreshCw } from "lucide-react";
import * as React from "react";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { screenShellClassName } from "@/lib/screen-layout";
import { useCurrency } from "@/lib/currency";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import { MOCK_TRANSACTIONS, type TransactionsList } from "@/mocks/transactions";
import { MOCK_OVERVIEW } from "@/mocks/seed";
import { useUiStore } from "@/store/ui";
import {
  NewTransactionDialog,
  createNewTransactionDraft,
  mockNewTransactionWalletSourceOptions,
  type NewTransactionDraft,
} from "@/components/transactions";
import { TransactionsTable } from "./TransactionsTable";
import { PeriodTabs, TransactionWorkbench } from "./TransactionWorkbench";
import {
  buildSwapCandidates,
  initialPeriodFromUrl,
  recordsForPeriod,
  sortTransactionsByDateDesc,
  toDashboardTransaction,
  transactionRecords,
  type FlowChartSelection,
  type PeriodKey,
  type SwapCandidateReference,
  type TableQuickFilter,
  type BreakdownSelection,
} from "./model";

const TransactionsDashboard = ({
  className,
  transactions = MOCK_TRANSACTIONS,
  nowRate = MOCK_OVERVIEW.priceEur,
  swapCandidates,
  swapCandidateTotal,
  isDataRefreshing = false,
  focusedTransaction,
  deepLinkedTransactionId,
  deepLinkedTransactionTab,
}: {
  className?: string;
  transactions?: TransactionsList;
  nowRate?: number | null;
  swapCandidates?: SwapCandidateReference[];
  swapCandidateTotal?: number | null;
  isDataRefreshing?: boolean;
  focusedTransaction?: TransactionsList["txs"][number] | null;
  deepLinkedTransactionId?: string | null;
  deepLinkedTransactionTab?: string;
}) => {
  const [period, setPeriod] = React.useState<PeriodKey>(initialPeriodFromUrl);
  const [newTxnOpen, setNewTxnOpen] = React.useState(false);
  const [flowChartSelection, setFlowChartSelection] =
    React.useState<FlowChartSelection | null>(null);
  const [quickFilter, setQuickFilter] =
    React.useState<TableQuickFilter | null>(null);
  const [breakdownSelection, setBreakdownSelection] =
    React.useState<BreakdownSelection | null>(null);
  const [resetTableFiltersToken, setResetTableFiltersToken] = React.useState(0);
  const [newTransactionDraft, setNewTransactionDraft] =
    React.useState<NewTransactionDraft>(createNewTransactionDraft);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const explorerSettings = useUiStore((s) => s.explorerSettings);
  const currency = useCurrency();
  const { syncAll, isSyncing } = useWalletSyncAction();
  const showRefreshSkeleton = isSyncing || isDataRefreshing;
  const records = React.useMemo(
    () => {
      const txs = transactions.txs.length ? [...transactions.txs] : [];
      if (
        focusedTransaction &&
        !txs.some(
          (tx) =>
            tx.id === focusedTransaction.id ||
            (Boolean(tx.externalId) &&
              tx.externalId === focusedTransaction.externalId) ||
            (Boolean(tx.explorerId) &&
              tx.explorerId === focusedTransaction.explorerId),
        )
      ) {
        txs.unshift(focusedTransaction);
      }
      return txs.length
        ? txs.map(toDashboardTransaction)
        : transactionRecords;
    },
    [focusedTransaction, transactions.txs],
  );
  const allPeriodRecords = React.useMemo(
    () => sortTransactionsByDateDesc(records),
    [records],
  );
  const periodRecords = React.useMemo(
    () =>
      period === "all"
        ? allPeriodRecords
        : recordsForPeriod(records, period),
    [allPeriodRecords, records, period],
  );
  const focusedRecord = React.useMemo(() => {
    if (!focusedTransaction) return null;
    return records.find(
      (record) =>
        record.id === focusedTransaction.id ||
        (Boolean(focusedTransaction.externalId) &&
          record.txnId === focusedTransaction.externalId) ||
        (Boolean(focusedTransaction.explorerId) &&
          record.explorerId === focusedTransaction.explorerId),
    ) ?? null;
  }, [focusedTransaction, records]);
  const tableRecords = React.useMemo(() => {
    if (
      !focusedRecord ||
      periodRecords.some((record) => record.id === focusedRecord.id)
    ) {
      return periodRecords;
    }
    return [focusedRecord, ...periodRecords];
  }, [focusedRecord, periodRecords]);
  const periodSwapCandidateIds = React.useMemo(
    () =>
      new Set(
        buildSwapCandidates(periodRecords, swapCandidates).flatMap((candidate) => [
          candidate.in.id,
          candidate.out.id,
        ]),
      ),
    [periodRecords, swapCandidates],
  );
  const handlePeriodChange = React.useCallback((nextPeriod: PeriodKey) => {
    setPeriod(nextPeriod);
    setFlowChartSelection(null);
    setQuickFilter(null);
    setBreakdownSelection(null);
    setResetTableFiltersToken((token) => token + 1);
  }, []);
  const resetTableFilters = React.useCallback(() => {
    setResetTableFiltersToken((token) => token + 1);
  }, []);

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    params.set("period", period);
    const nextQuery = params.toString();
    const nextUrl = nextQuery
      ? `${window.location.pathname}?${nextQuery}`
      : window.location.pathname;
    window.history.replaceState(null, "", nextUrl);
  }, [period]);

  return (
    <div
      className={cn(screenShellClassName, "relative", className)}
      aria-busy={showRefreshSkeleton}
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <PeriodTabs activePeriod={period} onPeriodChange={handlePeriodChange} />
        <div className="flex flex-wrap items-center gap-2 sm:gap-3">
          <Button
            variant="outline"
            size="sm"
            className="h-8 gap-2 sm:h-9"
            aria-label="Refresh watch-only connections"
            onClick={() => syncAll()}
            disabled={isSyncing}
          >
            <RefreshCw
              className={cn("size-4", isSyncing && "animate-spin")}
              aria-hidden="true"
            />
            <span className="hidden sm:inline">
              {isSyncing ? "Refreshing" : "Refresh"}
            </span>
          </Button>
          <NewTransactionDialog
            open={newTxnOpen}
            draft={newTransactionDraft}
            walletSourceOptions={mockNewTransactionWalletSourceOptions}
            onOpenChange={setNewTxnOpen}
            onDraftChange={setNewTransactionDraft}
            onSaveDraft={() => {
              setNewTxnOpen(false);
            }}
          />
        </div>
      </div>

      <TransactionWorkbench
        period={period}
        records={periodRecords}
        hideSensitive={hideSensitive}
        currency={currency}
        onFlowSelectionChange={setFlowChartSelection}
        onQuickFilterChange={setQuickFilter}
        onBreakdownSelectionChange={setBreakdownSelection}
        onTableFiltersReset={resetTableFilters}
        chartSelection={flowChartSelection}
        breakdownSelection={breakdownSelection}
        swapCandidateRefs={swapCandidates}
        swapCandidateTotal={swapCandidateTotal}
        isRefreshing={showRefreshSkeleton}
      />

      <TransactionsTable
        records={tableRecords}
        hideSensitive={hideSensitive}
        currency={currency}
        nowRate={nowRate}
        explorerSettings={explorerSettings}
        swapCandidateIds={periodSwapCandidateIds}
        chartSelection={flowChartSelection}
        quickFilter={quickFilter}
        breakdownSelection={breakdownSelection}
        onChartSelectionChange={setFlowChartSelection}
        onQuickFilterChange={setQuickFilter}
        onBreakdownSelectionChange={setBreakdownSelection}
        resetTableFiltersToken={resetTableFiltersToken}
        isRefreshing={showRefreshSkeleton}
        deepLinkedTransactionId={deepLinkedTransactionId}
        deepLinkedTransactionTab={deepLinkedTransactionTab}
      />
    </div>
  );
};

export { TransactionsDashboard };
