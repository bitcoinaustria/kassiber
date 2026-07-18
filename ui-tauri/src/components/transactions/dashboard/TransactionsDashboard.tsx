import { Download } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useDaemonMutation } from "@/daemon/client";
import { cn } from "@/lib/utils";
import { exportBasename, saveDaemonExport } from "@/lib/exportFile";
import {
  pageHeaderActionClassName,
  pageHeaderActionsClassName,
  screenShellClassName,
} from "@/lib/screen-layout";
import { useCurrency } from "@/lib/currency";
import { useWalletSyncAction } from "@/hooks/useWalletSyncAction";
import { MOCK_TRANSACTIONS, type TransactionsList } from "@/mocks/transactions";
import { MOCK_OVERVIEW } from "@/mocks/seed";
import {
  bookIdentityKey,
  type BookChartPeriod,
  useUiStore,
} from "@/store/ui";
import {
  DocumentImportDialog,
  NewTransactionDialog,
  createNewTransactionDraft,
  mockNewTransactionWalletSourceOptions,
  type NewTransactionDraft,
} from "@/components/transactions";
import {
  TransactionsTable,
  type TransactionTableFilterState,
} from "./TransactionsTable";
import { PeriodTabs, TransactionWorkbench } from "./TransactionWorkbench";
import {
  availablePeriodKeysForRecords,
  buildCandidateFlowOverrides,
  dashboardRecordsFromTxs,
  flowChartSelectionDateWindow,
  flowChartSelectionServerFlow,
  initialPeriodFromUrl,
  recordsForPeriod,
  resolveAutoPeriodForRecords,
  sortTransactionsByDateDesc,
  transactionListPeriodFilter,
  type FlowChartSelection,
  type PeriodKey,
  type ResolvedPeriodKey,
  type SwapCandidateReference,
  type TableQuickFilter,
  type BreakdownSelection,
} from "./model";

interface TransactionsExportResult {
  file?: string;
  rows?: number;
  format?: string;
  filename?: string;
}

function transactionPeriodFromSharedPeriod(
  period: BookChartPeriod | undefined,
): PeriodKey {
  return period ?? "1year";
}

const TransactionsDashboard = ({
  className,
  transactions = MOCK_TRANSACTIONS,
  tableTransactions,
  nowRate = MOCK_OVERVIEW.priceEur,
  pairingCandidateRefs,
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
  deepLinkedTransactionIds = [],
  onWalletScopeChange,
  onTableFilterArgsChange,
}: {
  className?: string;
  transactions?: TransactionsList;
  tableTransactions?: TransactionsList;
  nowRate?: number | null;
  pairingCandidateRefs?: SwapCandidateReference[];
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
  deepLinkedTransactionIds?: string[];
  onWalletScopeChange?: (wallet: string | null) => void;
  onTableFilterArgsChange?: (args: Record<string, unknown>) => void;
}) => {
  const { t } = useTranslation("transactions");
  const bookKey = useUiStore((state) => bookIdentityKey(state.identity));
  const storedBookChartPeriod = useUiStore((state) =>
    bookKey ? state.bookChartPeriods[bookKey] : undefined,
  );
  const setStoredBookChartPeriod = useUiStore(
    (state) => state.setBookChartPeriod,
  );
  const [period, setPeriod] = React.useState<PeriodKey>(() =>
    initialPeriodFromUrl(
      transactionPeriodFromSharedPeriod(storedBookChartPeriod),
    ),
  );
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
  const [tableExpanded, setTableExpanded] = React.useState(false);
  const [tableFilterState, setTableFilterState] =
    React.useState<TransactionTableFilterState>({
      status: "all",
      flow: "all",
      paymentMethod: "all",
      fee: "all",
      sort: null,
    });
  const [resetTableFiltersToken, setResetTableFiltersToken] = React.useState(0);
  const [newTransactionDraft, setNewTransactionDraft] =
    React.useState<NewTransactionDraft>(createNewTransactionDraft);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const explorerSettings = useUiStore((s) => s.explorerSettings);
  const dataMode = useUiStore((s) => s.dataMode);
  const currency = useCurrency();
  const { isSyncing } = useWalletSyncAction();
  const showRefreshSkeleton = isSyncing || isDataRefreshing;
  const addNotification = useUiStore((s) => s.addNotification);
  const previousBookKey = React.useRef(bookKey);
  const skipNextPeriodPersist = React.useRef(false);
  const exportTransactionsXlsx = useDaemonMutation<TransactionsExportResult>(
    "ui.transactions.export_xlsx",
  );
  const exportTransactionsCsv = useDaemonMutation<TransactionsExportResult>(
    "ui.transactions.export_csv",
  );
  const isExporting =
    exportTransactionsXlsx.isPending || exportTransactionsCsv.isPending;

  const handleExportTransactions = (format: "xlsx" | "csv") => {
    const mutation =
      format === "xlsx" ? exportTransactionsXlsx : exportTransactionsCsv;
    mutation.mutate(
      {},
      {
        onSuccess: async (envelope) => {
          const payload = envelope?.data;
          const exportPath = payload?.file ?? "";
          try {
            const { savedPath, copied } = await saveDaemonExport({
              exportPath,
              title: t("dashboard.export.saveTitle"),
              defaultName:
                format === "xlsx"
                  ? "kassiber-transactions.xlsx"
                  : "kassiber-transactions.csv",
              filters: [
                format === "xlsx"
                  ? { name: "Excel workbook", extensions: ["xlsx"] }
                  : { name: "CSV", extensions: ["csv"] },
              ],
            });
            addNotification({
              title: t("dashboard.export.done"),
              body: copied
                ? t("dashboard.export.savedTo", {
                    name: exportBasename(savedPath),
                  })
                : t("dashboard.export.rows", { count: payload?.rows ?? 0 }),
              tone: "success",
            });
          } catch (error) {
            addNotification({
              title: t("dashboard.export.failed"),
              body: error instanceof Error ? error.message : String(error),
              tone: "error",
            });
          }
        },
        onError: (error) => {
          addNotification({
            title: t("dashboard.export.failed"),
            body: error instanceof Error ? error.message : String(error),
            tone: "error",
          });
        },
      },
    );
  };
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
      return dashboardRecordsFromTxs(
        txs,
        t as (key: string, opts?: Record<string, unknown>) => string,
      );
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
    return dashboardRecordsFromTxs(
      txs,
      t as (key: string, opts?: Record<string, unknown>) => string,
    );
  }, [focusedTransaction, tableTransactions, transactions, t]);
  const allPeriodRecords = React.useMemo(
    () => sortTransactionsByDateDesc(records),
    [records],
  );
  // Offer only the period tabs the loaded transactions can actually chart.
  // Deriving these from the full history bounds (rather than the loaded rows)
  // let long-range tabs (5/10/15-year, "all") render silently truncated charts
  // on books larger than the workbench page cap, since the charts render from
  // the capped `records`. Gating on `records` keeps offered tabs and charted
  // data consistent; showing the full span for large books needs daemon-side
  // period aggregates, not an in-memory slice of the newest rows.
  const availablePeriods = React.useMemo(
    () => availablePeriodKeysForRecords(records),
    [records],
  );
  const periodOptions = React.useMemo<PeriodKey[]>(
    () => ["auto", ...availablePeriods],
    [availablePeriods],
  );
  const resolvedPeriod = React.useMemo<ResolvedPeriodKey>(
    () => resolveAutoPeriodForRecords(records, period),
    [period, records],
  );
  // In daemon-backed (real/regtest) mode the New Transaction picker must not
  // offer fabricated MOCK wallet names; derive single-wallet labels from the
  // loaded book instead (skipping synthesized "A → B" transfer strings).
  const realWalletSourceOptions = React.useMemo(() => {
    const labels = new Set<string>();
    for (const record of records) {
      const label = record.wallet;
      if (label && !label.includes("→")) labels.add(label);
    }
    return [...labels, "External"];
  }, [records]);
  const periodRecords = React.useMemo(
    () =>
      resolvedPeriod === "all"
        ? allPeriodRecords
        : recordsForPeriod(records, resolvedPeriod),
    [allPeriodRecords, records, resolvedPeriod],
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
      resolvedPeriod === "all"
        ? sortTransactionsByDateDesc(tableSourceRecords)
        : recordsForPeriod(tableSourceRecords, resolvedPeriod),
    [resolvedPeriod, tableSourceRecords],
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
  const visibleTableRecords = React.useMemo(() => {
    return tableRecords;
  }, [tableRecords]);
  const tableCandidateFlows = React.useMemo(
    () =>
      buildCandidateFlowOverrides(visibleTableRecords, pairingCandidateRefs),
    [visibleTableRecords, pairingCandidateRefs],
  );
  const handlePeriodChange = React.useCallback((nextPeriod: PeriodKey) => {
    // Period controls the data window only. Keep the user's active chart,
    // quick, breakdown, and table filters when that window changes.
    setPeriod(nextPeriod);
  }, []);
  React.useEffect(() => {
    if (previousBookKey.current === bookKey) return;
    previousBookKey.current = bookKey;
    skipNextPeriodPersist.current = true;
    setPeriod(
      initialPeriodFromUrl(
        transactionPeriodFromSharedPeriod(storedBookChartPeriod),
      ),
    );
    setFlowChartSelection(null);
    setQuickFilter(null);
    setBreakdownSelection(null);
    setResetTableFiltersToken((token) => token + 1);
  }, [bookKey, storedBookChartPeriod]);

  React.useEffect(() => {
    if (!bookKey) return;
    if (skipNextPeriodPersist.current) {
      skipNextPeriodPersist.current = false;
      return;
    }
    setStoredBookChartPeriod(bookKey, period);
  }, [bookKey, period, setStoredBookChartPeriod]);

  React.useEffect(() => {
    if (period === "auto") return;
    if (availablePeriods.includes(period)) return;
    handlePeriodChange(
      availablePeriods.includes("1year")
        ? "1year"
        : availablePeriods[availablePeriods.length - 1] ?? "all",
    );
  }, [availablePeriods, handlePeriodChange, period]);
  const resetTableFilters = React.useCallback(() => {
    setResetTableFiltersToken((token) => token + 1);
  }, []);

  // Keep the parent's wallet scope (which server-scopes the ui.transactions.list
  // queries) in lockstep with the wallet selection. Deriving it from
  // breakdownSelection means every clear path propagates — "Clear all", book
  // changes, and chart / quick-filter resets — not just the dropdown's own
  // clear, so the chip and the queried scope never disagree.
  React.useEffect(() => {
    onWalletScopeChange?.(
      breakdownSelection?.dimension === "wallet" &&
        breakdownSelection.match === "leg"
        ? breakdownSelection.key
        : null,
    );
  }, [breakdownSelection, onWalletScopeChange]);

  React.useEffect(() => {
    const args: Record<string, unknown> = {};
    const periodFilter = transactionListPeriodFilter(
      resolvedPeriod,
      deepLinkedTransactionIds,
    );
    if (periodFilter) args.period = periodFilter;
    if (deepLinkedTransactionIds.length > 0) {
      args.txids = deepLinkedTransactionIds;
    }

    if (flowChartSelection) {
      const window = flowChartSelectionDateWindow(flowChartSelection);
      if (window) {
        args.since = window.since;
        args.until = window.until;
        delete args.period;
      }
      const serverFlow = flowChartSelectionServerFlow(flowChartSelection);
      if (serverFlow) args.flow = serverFlow;
      if (flowChartSelection.mode === "external" && !flowChartSelection.segment) {
        args.quick = "external_flow";
      }
    }

    if (quickFilter) args.quick = quickFilter;
    if (breakdownSelection?.dimension === "network") {
      args.payment_method = breakdownSelection.key;
    }
    if (
      breakdownSelection?.dimension === "wallet" &&
      breakdownSelection.match !== "leg" &&
      !breakdownSelection.key.includes("→") &&
      !breakdownSelection.key.includes("->")
    ) {
      args.wallet = breakdownSelection.key;
    }

    if (tableFilterState.status !== "all") args.status = tableFilterState.status;
    if (tableFilterState.flow !== "all") args.flow = tableFilterState.flow;
    if (tableFilterState.paymentMethod !== "all") {
      args.payment_method = tableFilterState.paymentMethod;
    }
    if (tableFilterState.fee === "with-fees") args.withFees = true;
    if (tableFilterState.sort?.key === "date") {
      args.sort = "occurred-at";
      args.order = tableFilterState.sort.direction;
    } else if (tableFilterState.sort?.key === "amount") {
      args.sort = "amount";
      args.order = tableFilterState.sort.direction;
    }

    onTableFilterArgsChange?.(args);
  }, [
    breakdownSelection,
    deepLinkedTransactionIds,
    flowChartSelection,
    onTableFilterArgsChange,
    quickFilter,
    resolvedPeriod,
    tableFilterState,
  ]);

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

  React.useLayoutEffect(() => {
    if (!tableExpanded || typeof window === "undefined") return;
    const appMain = document.getElementById("app-main");
    if (!appMain) return;

    window.dispatchEvent(
      new CustomEvent("kassiber:assistant-dock-suppressed", {
        detail: { suppressed: true },
      }),
    );
    const previousOverflowY = appMain.style.overflowY;
    appMain.scrollTo({ top: 0 });
    appMain.style.overflowY = "hidden";

    return () => {
      window.dispatchEvent(
        new CustomEvent("kassiber:assistant-dock-suppressed", {
          detail: { suppressed: false },
        }),
      );
      appMain.style.overflowY = previousOverflowY;
    };
  }, [tableExpanded]);

  // Note: scrolling the table into view on a "Show all" deep link is handled by
  // TransactionsTable's own scroll-on-active-filter effect (the deep link seeds
  // breakdownSelection at mount), so no separate scroll is needed here.

  return (
    <div
      className={cn(
        screenShellClassName,
        tableExpanded &&
          "flex h-full min-h-0 flex-col overflow-hidden pt-0 pb-3 sm:pt-0 sm:pb-3 md:pt-0 md:pb-3",
        "relative",
        className,
      )}
      aria-busy={showRefreshSkeleton}
    >
      <div
        id="transactions-period-nav"
        className={cn(
          "-mx-3 flex flex-col bg-background px-3 sm:-mx-4 sm:flex-row sm:items-center sm:justify-between sm:px-4 md:-mx-5 md:px-5",
          tableExpanded
            ? "gap-2 pt-3 pb-0 sm:pt-4 sm:pb-0 md:pt-5 md:pb-0"
            : "sticky top-2 z-30 gap-2 py-0 shadow-[0_12px_18px_-18px_hsl(var(--foreground)/0.55)] before:pointer-events-none before:absolute before:inset-x-0 before:-top-2 before:h-2 before:bg-background before:content-[''] after:pointer-events-none after:absolute after:inset-x-0 after:-bottom-2 after:h-2 after:bg-background after:content-[''] sm:top-[0.6875rem] sm:before:-top-[0.6875rem] sm:before:h-[0.6875rem] sm:after:-bottom-[0.6875rem] sm:after:h-[0.6875rem] md:top-[0.8125rem] md:before:-top-[0.8125rem] md:before:h-[0.8125rem] md:after:-bottom-[0.8125rem] md:after:h-[0.8125rem]",
        )}
      >
        <PeriodTabs
          activePeriod={period}
          onPeriodChange={handlePeriodChange}
          periodOptions={periodOptions}
          resolvedPeriod={period === "auto" ? resolvedPeriod : null}
        />
        {tableExpanded ? (
          <div
            id="transactions-expanded-table-actions"
            className={cn(
              pageHeaderActionsClassName,
              "min-w-0 flex-1 justify-end",
            )}
          />
        ) : (
          <div className={pageHeaderActionsClassName}>
            <DocumentImportDialog />
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  variant="outline"
                  size="sm"
                  className={pageHeaderActionClassName}
                  aria-label={t("dashboard.export.label")}
                  disabled={isExporting}
                >
                  <Download className="size-4" aria-hidden="true" />
                  <span className="hidden sm:inline">
                    {t("dashboard.export.label")}
                  </span>
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem
                  onSelect={() => handleExportTransactions("xlsx")}
                >
                  {t("dashboard.export.xlsx")}
                </DropdownMenuItem>
                <DropdownMenuItem
                  onSelect={() => handleExportTransactions("csv")}
                >
                  {t("dashboard.export.csv")}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
            <NewTransactionDialog
              open={newTxnOpen}
              draft={newTransactionDraft}
              walletSourceOptions={
                dataMode === "mock"
                  ? mockNewTransactionWalletSourceOptions
                  : realWalletSourceOptions
              }
              movementCandidates={dataMode === "mock" ? undefined : []}
              onOpenChange={setNewTxnOpen}
              onDraftChange={setNewTransactionDraft}
              onSaveDraft={() => {
                setNewTxnOpen(false);
              }}
            />
          </div>
        )}
      </div>

      {!tableExpanded && (
        <TransactionWorkbench
          period={resolvedPeriod}
          records={periodRecords}
          hideSensitive={hideSensitive}
          currency={currency}
          onFlowSelectionChange={setFlowChartSelection}
          onQuickFilterChange={setQuickFilter}
          onBreakdownSelectionChange={setBreakdownSelection}
          onTableFiltersReset={resetTableFilters}
          chartSelection={flowChartSelection}
          pairingCandidateRefs={pairingCandidateRefs}
          swapCandidateTotal={swapCandidateTotal}
          isRefreshing={showRefreshSkeleton}
        />
      )}

      <div
        id="transactions-table"
        className={cn(
          "scroll-mt-4",
          tableExpanded && "min-h-0 flex-1 overflow-hidden",
        )}
      >
        <TransactionsTable
          records={visibleTableRecords}
          fullRecords={tableSourceRecords}
          hideSensitive={hideSensitive}
          currency={currency}
          nowRate={nowRate}
          explorerSettings={explorerSettings}
          swapCandidateIds={tableCandidateFlows.swapCandidateIds}
          transferCandidateIds={tableCandidateFlows.transferCandidateIds}
          chartSelection={flowChartSelection}
          quickFilter={quickFilter}
          breakdownSelection={breakdownSelection}
          transactionIdFilter={deepLinkedTransactionIds}
          onChartSelectionChange={setFlowChartSelection}
          onQuickFilterChange={setQuickFilter}
          onBreakdownSelectionChange={setBreakdownSelection}
          resetTableFiltersToken={resetTableFiltersToken}
          isRefreshing={showRefreshSkeleton}
          hasMoreRecords={hasMoreTransactions}
          isLoadingMoreRecords={isLoadingMoreTransactions}
          onLoadMoreRecords={onLoadMoreTransactions}
          onFilterStateChange={setTableFilterState}
          deepLinkedTransactionId={deepLinkedTransactionId}
          deepLinkedTransactionTab={deepLinkedTransactionTab}
          isExpanded={tableExpanded}
          onExpandedChange={setTableExpanded}
        />
      </div>
    </div>
  );
};

export { TransactionsDashboard };
