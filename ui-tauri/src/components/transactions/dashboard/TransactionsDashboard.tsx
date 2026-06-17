import { RefreshCw } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

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
  dashboardRecordsFromTxs,
  initialPeriodFromUrl,
  recordsForPeriod,
  sortTransactionsByDateDesc,
  type FlowChartSelection,
  type PeriodKey,
  type SwapCandidateReference,
  type TableQuickFilter,
  type BreakdownSelection,
} from "./model";

const workbenchBackedQuickFilters = new Set<TableQuickFilter>([
  "no_explorer_id",
  "missing_price",
  "failed_import",
]);

const TransactionsDashboard = ({
  className,
  transactions = MOCK_TRANSACTIONS,
  tableTransactions,
  nowRate = MOCK_OVERVIEW.priceEur,
  swapCandidates,
  swapCandidateTotal,
  isDataRefreshing = false,
  hasMoreTransactions = false,
  isLoadingMoreTransactions = false,
  onLoadMoreTransactions,
  focusedTransaction,
  deepLinkedTransactionId,
  deepLinkedTransactionTab,
  deepLinkedWallet,
  deepLinkedQuickFilter,
  onWalletScopeChange,
}: {
  className?: string;
  transactions?: TransactionsList;
  tableTransactions?: TransactionsList;
  nowRate?: number | null;
  swapCandidates?: SwapCandidateReference[];
  swapCandidateTotal?: number | null;
  isDataRefreshing?: boolean;
  hasMoreTransactions?: boolean;
  isLoadingMoreTransactions?: boolean;
  onLoadMoreTransactions?: () => void;
  focusedTransaction?: TransactionsList["txs"][number] | null;
  deepLinkedTransactionId?: string | null;
  deepLinkedTransactionTab?: string;
  deepLinkedWallet?: string | null;
  deepLinkedQuickFilter?: TableQuickFilter | null;
  onWalletScopeChange?: (wallet: string | null) => void;
}) => {
  const { t } = useTranslation("transactions");
  const [period, setPeriod] = React.useState<PeriodKey>(initialPeriodFromUrl);
  const [newTxnOpen, setNewTxnOpen] = React.useState(false);
  const [flowChartSelection, setFlowChartSelection] =
    React.useState<FlowChartSelection | null>(null);
  // Seed the table filters from a Wallet Detail deep link so "Show all" /
  // "Needs review" land pre-filtered to the wallet.
  const [quickFilter, setQuickFilter] = React.useState<TableQuickFilter | null>(
    deepLinkedQuickFilter ?? null,
  );
  const [breakdownSelection, setBreakdownSelection] =
    React.useState<BreakdownSelection | null>(
      deepLinkedWallet
        ? { dimension: "wallet", key: deepLinkedWallet, match: "leg" }
        : null,
    );
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
      return dashboardRecordsFromTxs(txs, t);
    },
    [focusedTransaction, transactions.txs, t],
  );
  const tableSourceRecords = React.useMemo(() => {
    const txs = (tableTransactions ?? transactions).txs.length
      ? [...(tableTransactions ?? transactions).txs]
      : [];
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
    return dashboardRecordsFromTxs(txs, t);
  }, [focusedTransaction, tableTransactions, transactions, t]);
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
    return tableSourceRecords.find(
      (record) =>
        record.id === focusedTransaction.id ||
        (Boolean(focusedTransaction.externalId) &&
          record.txnId === focusedTransaction.externalId) ||
        (Boolean(focusedTransaction.explorerId) &&
          record.explorerId === focusedTransaction.explorerId),
    ) ?? null;
  }, [focusedTransaction, tableSourceRecords]);
  const tablePeriodRecords = React.useMemo(
    () =>
      period === "all"
        ? sortTransactionsByDateDesc(tableSourceRecords)
        : recordsForPeriod(tableSourceRecords, period),
    [period, tableSourceRecords],
  );
  const tableRecords = React.useMemo(() => {
    if (
      !focusedRecord ||
      tablePeriodRecords.some((record) => record.id === focusedRecord.id)
    ) {
      return tablePeriodRecords;
    }
    return [focusedRecord, ...tablePeriodRecords];
  }, [focusedRecord, tablePeriodRecords]);
  const useWorkbenchRowsForTable =
    quickFilter !== null && workbenchBackedQuickFilters.has(quickFilter);
  const visibleTableRecords = React.useMemo(() => {
    if (!useWorkbenchRowsForTable) return tableRecords;
    if (
      !focusedRecord ||
      periodRecords.some((record) => record.id === focusedRecord.id)
    ) {
      return periodRecords;
    }
    return [focusedRecord, ...periodRecords];
  }, [focusedRecord, periodRecords, tableRecords, useWorkbenchRowsForTable]);
  const tableSwapCandidateIds = React.useMemo(
    () =>
      new Set(
        buildSwapCandidates(visibleTableRecords, swapCandidates).flatMap(
          (candidate) => [candidate.in.id, candidate.out.id],
        ),
      ),
    [visibleTableRecords, swapCandidates],
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

  // Keep the parent's wallet scope (which server-scopes the ui.transactions.list
  // queries) in lockstep with the wallet selection. Deriving it from
  // breakdownSelection means EVERY clear path propagates — "Clear all", period
  // change, chart / quick-filter resets — not just the dropdown's own clear, so
  // the chip and the queried scope never disagree.
  React.useEffect(() => {
    onWalletScopeChange?.(
      breakdownSelection?.dimension === "wallet" &&
        breakdownSelection.match === "leg"
        ? breakdownSelection.key
        : null,
    );
  }, [breakdownSelection, onWalletScopeChange]);

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

  // Note: scrolling the table into view on a "Show all" deep link is handled by
  // TransactionsTable's own scroll-on-active-filter effect (the deep link seeds
  // breakdownSelection at mount), so no separate scroll is needed here.

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
            aria-label={t("dashboard.refreshBook")}
            onClick={() => syncAll()}
            disabled={isSyncing}
          >
            <RefreshCw
              className={cn("size-4", isSyncing && "animate-spin")}
              aria-hidden="true"
            />
            <span className="hidden sm:inline">
              {isSyncing ? t("dashboard.refreshing") : t("dashboard.refresh")}
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

      <div id="transactions-table" className="scroll-mt-4">
        <TransactionsTable
          records={visibleTableRecords}
          hideSensitive={hideSensitive}
          currency={currency}
          nowRate={nowRate}
          explorerSettings={explorerSettings}
          swapCandidateIds={tableSwapCandidateIds}
          chartSelection={flowChartSelection}
          quickFilter={quickFilter}
          breakdownSelection={breakdownSelection}
          onChartSelectionChange={setFlowChartSelection}
          onQuickFilterChange={setQuickFilter}
          onBreakdownSelectionChange={setBreakdownSelection}
          resetTableFiltersToken={resetTableFiltersToken}
          isRefreshing={showRefreshSkeleton}
          hasMoreRecords={hasMoreTransactions && !useWorkbenchRowsForTable}
          isLoadingMoreRecords={isLoadingMoreTransactions}
          onLoadMoreRecords={onLoadMoreTransactions}
          deepLinkedTransactionId={deepLinkedTransactionId}
          deepLinkedTransactionTab={deepLinkedTransactionTab}
        />
      </div>
    </div>
  );
};

export { TransactionsDashboard };
