import { Download } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { useNavigate } from "@tanstack/react-router";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
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
import { TransactionsTable } from "./TransactionsTable";
import { PeriodTabs, TransactionWorkbench } from "./TransactionWorkbench";
import {
  availablePeriodKeysForRecords,
  availablePeriodKeysForHistory,
  buildCandidateFlowOverrides,
  buildTransactionListFilterArgs,
  DEFAULT_TRANSACTION_TABLE_FILTER_STATE,
  dashboardRecordsFromTxs,
  initialPeriodFromUrl,
  recordsForPeriod,
  resolveAutoPeriodForRecords,
  sortTransactionsByDateDesc,
  serializeTransactionFilterParams,
  type FlowChartSelection,
  type PeriodKey,
  type ResolvedPeriodKey,
  type SwapCandidateReference,
  type TableQuickFilter,
  type BreakdownSelection,
  type TransactionFilterState,
  type TransactionScopeParams,
  transactionPeriodDateWindow,
  type TransactionDashboardSnapshot,
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
  scopeParams,
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
  scopeParams: TransactionScopeParams;
  onWalletScopeChange?: (wallet: string | null) => void;
  onTableFilterArgsChange?: (args: Record<string, unknown>) => void;
}) => {
  const { t } = useTranslation("transactions");
  const navigate = useNavigate();
  const bookKey = useUiStore((state) => bookIdentityKey(state.identity));
  const storedBookChartPeriod = useUiStore((state) =>
    bookKey ? state.bookChartPeriods[bookKey] : undefined,
  );
  const setStoredBookChartPeriod = useUiStore(
    (state) => state.setBookChartPeriod,
  );
  const initialPeriod =
    scopeParams.period ??
    initialPeriodFromUrl(transactionPeriodFromSharedPeriod(storedBookChartPeriod));
  const [filterState, patchFilterState] = React.useReducer(
    (state: TransactionFilterState, patch: Partial<TransactionFilterState>) => ({
      ...state,
      ...patch,
    }),
    {
      period: initialPeriod,
      flowChartSelection: scopeParams.flowChartSelection,
      quickFilter: scopeParams.quick,
      breakdownSelection: scopeParams.breakdownSelection,
      transactionIds: scopeParams.transactionIds,
      table: scopeParams.table,
    },
  );
  const {
    period,
    flowChartSelection,
    quickFilter,
    breakdownSelection,
    transactionIds,
    table: tableFilterState,
  } = filterState;
  const setFlowChartSelection = React.useCallback(
    (value: FlowChartSelection | null) => patchFilterState({ flowChartSelection: value }),
    [],
  );
  const setQuickFilter = React.useCallback(
    (value: TableQuickFilter | null) => patchFilterState({ quickFilter: value }),
    [],
  );
  const setBreakdownSelection = React.useCallback(
    (value: BreakdownSelection | null) => patchFilterState({ breakdownSelection: value }),
    [],
  );
  const [newTxnOpen, setNewTxnOpen] = React.useState(false);
  const [tableExpanded, setTableExpanded] = React.useState(false);
  const [newTransactionDraft, setNewTransactionDraft] =
    React.useState<NewTransactionDraft>(createNewTransactionDraft);
  const hideSensitive = useUiStore((s) => s.hideSensitive);
  const explorerSettings = useUiStore((s) => s.explorerSettings);
  const dataMode = useUiStore((s) => s.dataMode);
  const currency = useCurrency();
  const { isSyncing } = useWalletSyncAction();
  const baseRefreshSkeleton = isSyncing || isDataRefreshing;
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
  // The aggregate endpoint owns full-history bounds in real mode, so long-range
  // tabs never imply that a capped client page represents the whole book.
  // Mock mode keeps deriving the options from its in-memory records.
  const resolvedPeriod = React.useMemo<ResolvedPeriodKey>(
    () => resolveAutoPeriodForRecords(records, period),
    [period, records],
  );
  const dashboardWindow = React.useMemo(
    () => transactionPeriodDateWindow(resolvedPeriod),
    [resolvedPeriod],
  );
  const dashboardWallet =
    breakdownSelection?.dimension === "wallet" &&
    breakdownSelection.match === "leg"
      ? breakdownSelection.key
      : null;
  const dashboardQuery = useDaemon<TransactionDashboardSnapshot>(
    "ui.transactions.dashboard",
    {
      period: resolvedPeriod,
      ...(dashboardWindow ?? {}),
      timezone: Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC",
      ...(dashboardWallet ? { wallet: dashboardWallet } : {}),
    },
    { enabled: dataMode !== "mock" },
  );
  const dashboardSnapshot =
    dashboardQuery.data?.kind === "ui.transactions.dashboard"
      ? dashboardQuery.data.data
      : null;
  const showRefreshSkeleton = baseRefreshSkeleton || dashboardQuery.isFetching;
  const availablePeriods = React.useMemo(
    () =>
      dashboardSnapshot
        ? availablePeriodKeysForHistory(
            dashboardSnapshot.history.earliest,
            dashboardSnapshot.history.latest,
          )
        : availablePeriodKeysForRecords(records),
    [dashboardSnapshot, records],
  );
  const periodOptions = React.useMemo<PeriodKey[]>(
    () => ["auto", ...availablePeriods],
    [availablePeriods],
  );
  const effectiveCandidateRefs =
    dashboardSnapshot?.candidates ?? pairingCandidateRefs;
  // In daemon-backed (real/regtest) mode the New Transaction picker must not
  // offer fabricated MOCK wallet names; derive single-wallet labels from the
  // loaded book instead (skipping synthesized "A → B" transfer strings).
  const realWalletSourceOptions = React.useMemo(() => {
    const labels = new Set<string>();
    for (const label of dashboardSnapshot?.history.walletOptions ?? []) {
      if (label && !label.includes("→")) labels.add(label);
    }
    if (labels.size === 0) {
      for (const record of records) {
        const label = record.wallet;
        if (label && !label.includes("→")) labels.add(label);
      }
    }
    return [...labels, "External"];
  }, [dashboardSnapshot, records]);
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
  // Real table pages are already scoped by the canonical daemon request. A
  // second client-side period pass would erase exact txid/chart-bucket results
  // that intentionally override the broad period (the original empty-table
  // failure). Mock mode still needs the in-memory period behavior.
  const tablePeriodRecords = React.useMemo(
    () =>
      dataMode !== "mock" || resolvedPeriod === "all"
        ? sortTransactionsByDateDesc(tableSourceRecords)
        : recordsForPeriod(tableSourceRecords, resolvedPeriod),
    [dataMode, resolvedPeriod, tableSourceRecords],
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
  const tableCandidateFlows = React.useMemo(
    () => buildCandidateFlowOverrides(tableRecords, effectiveCandidateRefs),
    [tableRecords, effectiveCandidateRefs],
  );
  const handlePeriodChange = React.useCallback((nextPeriod: PeriodKey) => {
    // Period controls the data window only. Keep the user's active chart,
    // quick, breakdown, and table filters when that window changes.
    patchFilterState({ period: nextPeriod });
  }, []);
  React.useEffect(() => {
    patchFilterState({
      ...(scopeParams.period ? { period: scopeParams.period } : {}),
      flowChartSelection: scopeParams.flowChartSelection,
      quickFilter: scopeParams.quick,
      breakdownSelection: scopeParams.breakdownSelection,
      transactionIds: scopeParams.transactionIds,
      table: scopeParams.table,
    });
  }, [scopeParams]);
  React.useEffect(() => {
    if (previousBookKey.current === bookKey) return;
    previousBookKey.current = bookKey;
    skipNextPeriodPersist.current = true;
    patchFilterState({
      period:
        scopeParams.period ??
        transactionPeriodFromSharedPeriod(storedBookChartPeriod),
      flowChartSelection: null,
      quickFilter: null,
      breakdownSelection: null,
      transactionIds: [],
      table: { ...DEFAULT_TRANSACTION_TABLE_FILTER_STATE },
    });
  }, [bookKey, scopeParams.period, storedBookChartPeriod]);

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
    patchFilterState({ table: { ...DEFAULT_TRANSACTION_TABLE_FILTER_STATE } });
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
    onTableFilterArgsChange?.(
      buildTransactionListFilterArgs({
        period: resolvedPeriod,
        transactionIds,
        flowChartSelection,
        quickFilter,
        breakdownSelection,
        tableFilterState,
        pairingCandidateRefs: effectiveCandidateRefs,
      }),
    );
  }, [
    breakdownSelection,
    transactionIds,
    flowChartSelection,
    effectiveCandidateRefs,
    onTableFilterArgsChange,
    quickFilter,
    resolvedPeriod,
    tableFilterState,
  ]);

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const nextQuery = serializeTransactionFilterParams(
      window.location.search,
      filterState,
    );
    const nextUrl = nextQuery
      ? `${window.location.pathname}?${nextQuery}`
      : window.location.pathname;
    if (`${window.location.pathname}${window.location.search}` === nextUrl) return;
    void navigate({ to: nextUrl, replace: true });
  }, [filterState, navigate]);

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
          pairingCandidateRefs={effectiveCandidateRefs}
          swapCandidateTotal={dashboardSnapshot?.swapCandidateTotal ?? swapCandidateTotal}
          dashboardSnapshot={dashboardSnapshot}
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
          records={tableRecords}
          transactionSetRecords={tableSourceRecords}
          hideSensitive={hideSensitive}
          currency={currency}
          nowRate={nowRate}
          explorerSettings={explorerSettings}
          swapCandidateIds={tableCandidateFlows.swapCandidateIds}
          transferCandidateIds={tableCandidateFlows.transferCandidateIds}
          chartSelection={flowChartSelection}
          quickFilter={quickFilter}
          breakdownSelection={breakdownSelection}
          transactionIdFilter={transactionIds}
          onTransactionIdFilterChange={(ids) => patchFilterState({ transactionIds: ids })}
          onChartSelectionChange={setFlowChartSelection}
          onQuickFilterChange={setQuickFilter}
          onBreakdownSelectionChange={setBreakdownSelection}
          isRefreshing={showRefreshSkeleton}
          hasMoreRecords={hasMoreTransactions}
          isLoadingMoreRecords={isLoadingMoreTransactions}
          onLoadMoreRecords={onLoadMoreTransactions}
          filterState={tableFilterState}
          onFilterStateChange={(table) => patchFilterState({ table })}
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
