import {
  ArrowDownRight,
  ArrowLeftRight,
  ArrowUpRight,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  Copy,
  Eye,
  ExternalLink,
  Filter,
  MoreHorizontal,
  Pencil,
  Wallet,
  X,
} from "lucide-react";
import * as React from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { cn } from "@/lib/utils";
import { type Currency } from "@/lib/currency";
import { type ExplorerSettings } from "@/lib/explorer";
import { TransactionDetailController } from "./TransactionDetailController";
import {
  ExplorerOpenDialog,
  allPaymentMethods,
  allTransactionFlows,
  allTransactionStatuses,
  austrianTaxClassificationFor,
  blurClass,
  copyText,
  currencyFormatter,
  draftForTransaction,
  explorerForTransaction,
  formatCounterDisplayMoney,
  formatDisplayMoney,
  formatShortTxid,
  formatSignedDisplayMoney,
  pricingSelectionValue,
  pricingSourceLabel,
  pricingSourceStyles,
  transactionBtc,
  transactionFlow,
  transactionFlowLabels,
  transactionFlowStyles,
  transactionStatusIcons,
  transactionStatusLabels,
  transactionStatusStyles,
  type Transaction,
  type TransactionFlow,
  type TransactionStatus,
} from "@/components/transactions";
import {
  PAGE_SIZE_OPTIONS,
  breakdownSelectionLabel,
  dateFilterOptions,
  filterChipClassName,
  flowChartSelectionLabel,
  isRedundantTransactionLabel,
  matchesFlowChartSelection,
  matchesTransactionDeepLink,
  pairRailLabel,
  quickFilterLabel,
  readTransactionDetailParams,
  updateTransactionDetailParams,
  type BreakdownSelection,
  type FeeFilter,
  type FlowChartSelection,
  type TableQuickFilter,
} from "./model";

const TransactionsTable = ({
  records,
  hideSensitive,
  currency,
  nowRate,
  explorerSettings,
  swapCandidateIds = new Set<string>(),
  chartSelection,
  quickFilter,
  breakdownSelection,
  onChartSelectionChange,
  onQuickFilterChange,
  onBreakdownSelectionChange,
  resetTableFiltersToken,
  isRefreshing,
}: {
  records: Transaction[];
  hideSensitive: boolean;
  currency: Currency;
  nowRate: number | null;
  explorerSettings: ExplorerSettings;
  swapCandidateIds?: Set<string>;
  chartSelection: FlowChartSelection | null;
  quickFilter: TableQuickFilter | null;
  breakdownSelection: BreakdownSelection | null;
  onChartSelectionChange: (selection: FlowChartSelection | null) => void;
  onQuickFilterChange: (filter: TableQuickFilter | null) => void;
  onBreakdownSelectionChange: (selection: BreakdownSelection | null) => void;
  resetTableFiltersToken: number;
  isRefreshing?: boolean;
}) => {
  const [statusFilter, setStatusFilter] = React.useState<string>("all");
  const [dateFilter, setDateFilter] = React.useState<string>("all");
  const [flowFilter, setFlowFilter] = React.useState<string>("all");
  const [paymentMethodFilter, setPaymentMethodFilter] =
    React.useState<string>("all");
  const [feeFilter, setFeeFilter] = React.useState<FeeFilter>("all");
  const [currentPage, setCurrentPage] = React.useState(1);
  const [pageSize, setPageSize] = React.useState(10);
  const [isHydrated, setIsHydrated] = React.useState(false);
  const [explorerTransaction, setExplorerTransaction] =
    React.useState<Transaction | null>(null);
  const [detailTransaction, setDetailTransaction] =
    React.useState<Transaction | null>(null);
  const [detailInitialTab, setDetailInitialTab] = React.useState("details");
  const pendingDetailLinkRef = React.useRef(readTransactionDetailParams());
  const tableRef = React.useRef<HTMLDivElement>(null);
  const explorerTarget = explorerTransaction
    ? explorerForTransaction(explorerTransaction, explorerSettings)
    : null;
  const displayFlow = React.useCallback(
    (txn: Transaction): TransactionFlow =>
      swapCandidateIds.has(txn.id) ? "swap" : transactionFlow(txn),
    [swapCandidateIds],
  );
  const getDraft = React.useCallback(
    (txn: Transaction) => draftForTransaction(txn),
    [],
  );
  const openTransactionDetail = React.useCallback(
    (txn: Transaction, tab = "details") => {
      setDetailInitialTab(tab);
      setDetailTransaction(txn);
      updateTransactionDetailParams(txn.id, tab);
    },
    [],
  );

  const hasActiveFilters =
    chartSelection !== null ||
    quickFilter !== null ||
    breakdownSelection !== null ||
    statusFilter !== "all" ||
    dateFilter !== "all" ||
    flowFilter !== "all" ||
    paymentMethodFilter !== "all" ||
    feeFilter !== "all";

  const clearFilters = () => {
    onChartSelectionChange(null);
    onQuickFilterChange(null);
    onBreakdownSelectionChange(null);
    setStatusFilter("all");
    setDateFilter("all");
    setFlowFilter("all");
    setPaymentMethodFilter("all");
    setFeeFilter("all");
  };

  React.useEffect(() => {
    if (resetTableFiltersToken === 0) return;
    setStatusFilter("all");
    setDateFilter("all");
    setFlowFilter("all");
    setPaymentMethodFilter("all");
    setFeeFilter("all");
  }, [resetTableFiltersToken]);

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);

    const nextStatus = params.get("status");
    if (
      nextStatus &&
      (nextStatus === "all" ||
        allTransactionStatuses.includes(nextStatus as TransactionStatus))
    ) {
      setStatusFilter(nextStatus);
    }

    const nextDate = params.get("date");
    if (
      nextDate &&
      dateFilterOptions.some((option) => option.value === nextDate)
    ) {
      setDateFilter(nextDate);
    }

    const nextFlow = params.get("flow");
    if (
      nextFlow &&
      (nextFlow === "all" ||
        allTransactionFlows.includes(nextFlow as TransactionFlow))
    ) {
      setFlowFilter(nextFlow);
    }

    const nextPayment = params.get("payment");
    if (
      nextPayment &&
      (nextPayment === "all" ||
        allPaymentMethods.includes(
          nextPayment as (typeof allPaymentMethods)[number],
        ))
    ) {
      setPaymentMethodFilter(nextPayment);
    }

    const nextFees = params.get("fees");
    if (nextFees === "with-fees" || nextFees === "true" || nextFees === "1") {
      setFeeFilter("with-fees");
    } else if (nextFees === "all") {
      setFeeFilter("all");
    }

    const nextPage = Number(params.get("page"));
    if (!Number.isNaN(nextPage) && nextPage > 0) {
      setCurrentPage(nextPage);
    }

    const nextPageSize = Number(params.get("pageSize"));
    if (
      !Number.isNaN(nextPageSize) &&
      PAGE_SIZE_OPTIONS.includes(nextPageSize)
    ) {
      setPageSize(nextPageSize);
    }

    setIsHydrated(true);
  }, []);

  React.useEffect(() => {
    const pending = pendingDetailLinkRef.current;
    if (!pending.transactionId) return;
    const transaction = records.find((txn) =>
      matchesTransactionDeepLink(txn, pending.transactionId ?? ""),
    );
    if (!transaction) return;
    pendingDetailLinkRef.current = { transactionId: null, tab: "details" };
    openTransactionDetail(transaction, pending.tab);
  }, [records, openTransactionDetail]);

  const filteredTransactions = React.useMemo(() => {
    return records.filter((txn) => {
      const draft = getDraft(txn);
      const matchesStatus =
        statusFilter === "all" || draft.reviewStatus === statusFilter;

      const matchesFlow =
        flowFilter === "all" || displayFlow(txn) === flowFilter;

      const matchesPaymentMethod =
        paymentMethodFilter === "all" ||
        txn.paymentMethod === paymentMethodFilter;

      const matchesFees =
        feeFilter === "all" || (txn.feeBtc ?? 0) > 0 || (txn.feeEur ?? 0) > 0;

      const matchesChartSelection =
        !chartSelection ||
        matchesFlowChartSelection(txn, chartSelection, displayFlow);

      const matchesQuickFilter =
        quickFilter === null ||
        (quickFilter === "external_flow" &&
          ["incoming", "outgoing"].includes(displayFlow(txn))) ||
        (quickFilter === "review_queue" && draft.reviewStatus !== "completed");

      const matchesBreakdownSelection =
        !breakdownSelection ||
        (breakdownSelection.dimension === "network" &&
          txn.paymentMethod === breakdownSelection.key) ||
        (breakdownSelection.dimension === "wallet" &&
          (txn.wallet ?? "Unassigned") === breakdownSelection.key);

      let matchesDate = true;
      const pd = txn.date.toLowerCase();
      switch (dateFilter) {
        case "today":
          matchesDate = pd === "today";
          break;
        case "yesterday":
          matchesDate = pd === "1 day ago";
          break;
        case "7days":
          matchesDate =
            pd === "today" ||
            pd.includes("day ago") ||
            (pd.includes("days ago") && parseInt(pd) <= 7);
          break;
        case "30days":
          matchesDate =
            pd === "today" ||
            pd.includes("day ago") ||
            (pd.includes("days ago") && parseInt(pd) <= 30);
          break;
      }

      return (
        matchesChartSelection &&
        matchesQuickFilter &&
        matchesBreakdownSelection &&
        matchesStatus &&
        matchesFlow &&
        matchesPaymentMethod &&
        matchesFees &&
        matchesDate
      );
    });
  }, [
    records,
    getDraft,
    chartSelection,
    quickFilter,
    breakdownSelection,
    statusFilter,
    dateFilter,
    flowFilter,
    paymentMethodFilter,
    feeFilter,
    displayFlow,
  ]);

  React.useEffect(() => {
    if (
      (!chartSelection && !quickFilter && !breakdownSelection) ||
      typeof window === "undefined"
    ) {
      return;
    }
    window.requestAnimationFrame(() => {
      tableRef.current?.scrollIntoView({
        behavior: "smooth",
        block: "start",
      });
    });
  }, [chartSelection, quickFilter, breakdownSelection]);

  const totalPages = Math.ceil(filteredTransactions.length / pageSize);

  const paginatedTransactions = React.useMemo(() => {
    const startIndex = (currentPage - 1) * pageSize;
    const endIndex = startIndex + pageSize;
    return filteredTransactions.slice(startIndex, endIndex);
  }, [filteredTransactions, currentPage, pageSize]);

  React.useEffect(() => {
    setCurrentPage(1);
  }, [
    chartSelection,
    quickFilter,
    breakdownSelection,
    statusFilter,
    dateFilter,
    flowFilter,
    paymentMethodFilter,
    feeFilter,
    pageSize,
  ]);

  React.useEffect(() => {
    if (!isHydrated || typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    params.delete("q");

    if (statusFilter !== "all") {
      params.set("status", statusFilter);
    } else {
      params.delete("status");
    }

    if (dateFilter !== "all") {
      params.set("date", dateFilter);
    } else {
      params.delete("date");
    }

    if (flowFilter !== "all") {
      params.set("flow", flowFilter);
    } else {
      params.delete("flow");
    }

    if (paymentMethodFilter !== "all") {
      params.set("payment", paymentMethodFilter);
    } else {
      params.delete("payment");
    }

    if (feeFilter === "with-fees") {
      params.set("fees", feeFilter);
    } else {
      params.delete("fees");
    }

    if (currentPage > 1) {
      params.set("page", String(currentPage));
    } else {
      params.delete("page");
    }

    if (pageSize !== PAGE_SIZE_OPTIONS[0]) {
      params.set("pageSize", String(pageSize));
    } else {
      params.delete("pageSize");
    }

    const nextQuery = params.toString();
    const nextUrl = nextQuery
      ? `${window.location.pathname}?${nextQuery}`
      : window.location.pathname;
    window.history.replaceState(null, "", nextUrl);
  }, [
    statusFilter,
    dateFilter,
    flowFilter,
    paymentMethodFilter,
    feeFilter,
    currentPage,
    pageSize,
    isHydrated,
  ]);

  const goToPage = (page: number) => {
    setCurrentPage(Math.max(1, Math.min(page, totalPages)));
  };

  return (
    <>
      <div
        ref={tableRef}
        className="rounded-xl border bg-card"
        role={isRefreshing ? "status" : undefined}
        aria-live={isRefreshing ? "polite" : undefined}
      >
      <div className="flex flex-col gap-3 p-3 sm:flex-row sm:items-center sm:gap-4 sm:px-6 sm:py-3.5">
        <div className="flex flex-1 items-center gap-2">
          <span className="text-sm font-medium sm:text-base">Transactions</span>
          <span className="ml-1 inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-[10px] font-medium text-gray-600 ring-1 ring-gray-500/10 ring-inset sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20">
            {isRefreshing ? (
              <Skeleton className="h-3 w-5" />
            ) : (
              filteredTransactions.length
            )}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <Select value={dateFilter} onValueChange={setDateFilter}>
            <SelectTrigger
              className="h-8 w-[120px] text-xs sm:h-9 sm:w-[140px] sm:text-sm"
              aria-label="Filter by date"
            >
              <SelectValue placeholder="Date" />
            </SelectTrigger>
            <SelectContent>
              {dateFilterOptions.map((option) => (
                <SelectItem key={option.value} value={option.value}>
                  {option.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className={cn(
                  "h-8 gap-1.5 sm:h-9 sm:gap-2",
                  statusFilter !== "all" && "border-primary",
                )}
                aria-label="Filter by status"
              >
                <Filter className="size-3.5 sm:size-4" aria-hidden="true" />
                <span className="hidden sm:inline">Status</span>
                {statusFilter !== "all" && (
                  <span className="size-1.5 rounded-full bg-primary sm:size-2" />
                )}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[180px]">
              <DropdownMenuLabel>Filter by Status</DropdownMenuLabel>
              <DropdownMenuCheckboxItem
                checked={statusFilter === "all"}
                onCheckedChange={() => setStatusFilter("all")}
              >
                All Statuses
              </DropdownMenuCheckboxItem>
              {allTransactionStatuses.map((status) => (
                <DropdownMenuCheckboxItem
                  key={status}
                  checked={statusFilter === status}
                  onCheckedChange={() => setStatusFilter(status)}
                >
                  {transactionStatusLabels[status]}
                </DropdownMenuCheckboxItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className={cn(
                  "h-8 gap-1.5 sm:h-9 sm:gap-2",
                  flowFilter !== "all" && "border-primary",
                )}
                aria-label="Filter by flow"
              >
                <ArrowLeftRight
                  className="size-3.5 sm:size-4"
                  aria-hidden="true"
                />
                <span className="hidden sm:inline">Flow</span>
                {flowFilter !== "all" && (
                  <span className="size-1.5 rounded-full bg-primary sm:size-2" />
                )}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[190px]">
              <DropdownMenuLabel>Filter by flow</DropdownMenuLabel>
              <DropdownMenuCheckboxItem
                checked={flowFilter === "all"}
                onCheckedChange={() => setFlowFilter("all")}
              >
                All flows
              </DropdownMenuCheckboxItem>
              {allTransactionFlows.map((flow) => (
                <DropdownMenuCheckboxItem
                  key={flow}
                  checked={flowFilter === flow}
                  onCheckedChange={() => setFlowFilter(flow)}
                >
                  {transactionFlowLabels[flow]}
                </DropdownMenuCheckboxItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>

          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className={cn(
                  "h-8 gap-1.5 sm:h-9 sm:gap-2",
                  paymentMethodFilter !== "all" && "border-primary",
                )}
                aria-label="Filter by payment method"
              >
                <Wallet className="size-3.5 sm:size-4" aria-hidden="true" />
                <span className="hidden sm:inline">Network</span>
                {paymentMethodFilter !== "all" && (
                  <span className="size-1.5 rounded-full bg-primary sm:size-2" />
                )}
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[200px]">
              <DropdownMenuLabel>Filter by network</DropdownMenuLabel>
              <DropdownMenuCheckboxItem
                checked={paymentMethodFilter === "all"}
                onCheckedChange={() => setPaymentMethodFilter("all")}
              >
                All networks
              </DropdownMenuCheckboxItem>
              {allPaymentMethods.map((method) => (
                <DropdownMenuCheckboxItem
                  key={method}
                  checked={paymentMethodFilter === method}
                  onCheckedChange={() => setPaymentMethodFilter(method)}
                >
                  {method}
                </DropdownMenuCheckboxItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      {hasActiveFilters && (
        <div className="flex flex-wrap items-center gap-2 px-3 pb-3 sm:px-6">
          <span className="text-[10px] text-muted-foreground sm:text-xs">
            Filters:
          </span>
          {chartSelection && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => onChartSelectionChange(null)}
              aria-label={`Clear chart filter ${flowChartSelectionLabel(chartSelection)}`}
            >
              Chart: {flowChartSelectionLabel(chartSelection)}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {quickFilter && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => onQuickFilterChange(null)}
              aria-label={`Clear ${quickFilterLabel(quickFilter)} filter`}
            >
              {quickFilterLabel(quickFilter)}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {breakdownSelection && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => onBreakdownSelectionChange(null)}
              aria-label={`Clear ${breakdownSelectionLabel(breakdownSelection)} filter`}
            >
              {breakdownSelectionLabel(breakdownSelection)}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {statusFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setStatusFilter("all")}
              aria-label={`Clear ${transactionStatusLabels[statusFilter as TransactionStatus]} filter`}
            >
              {transactionStatusLabels[statusFilter as TransactionStatus]}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {dateFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setDateFilter("all")}
              aria-label={`Clear ${dateFilterOptions.find((o) => o.value === dateFilter)?.label} filter`}
            >
              {dateFilterOptions.find((o) => o.value === dateFilter)?.label}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {flowFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setFlowFilter("all")}
              aria-label={`Clear ${transactionFlowLabels[flowFilter as TransactionFlow]} filter`}
            >
              {transactionFlowLabels[flowFilter as TransactionFlow]}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {paymentMethodFilter !== "all" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setPaymentMethodFilter("all")}
              aria-label={`Clear ${paymentMethodFilter} filter`}
            >
              {paymentMethodFilter}
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          {feeFilter === "with-fees" && (
            <button
              type="button"
              className={filterChipClassName}
              onClick={() => setFeeFilter("all")}
              aria-label="Clear with fees filter"
            >
              With fees
              <X className="size-2.5 sm:size-3" aria-hidden="true" />
            </button>
          )}
          <button
            onClick={clearFilters}
            className="text-[10px] text-destructive hover:underline sm:text-xs"
          >
            Clear all
          </button>
        </div>
      )}

      <div className="overflow-x-auto px-3 pb-3 sm:px-6 sm:pb-4">
        <Table className="min-w-[980px]">
          <TableHeader>
            <TableRow className="bg-muted/50 hover:bg-muted/50">
              <TableHead className="min-w-[280px] text-xs font-medium text-muted-foreground sm:text-sm">
                Transaction
              </TableHead>
              <TableHead className="min-w-[140px] text-right text-xs font-medium text-muted-foreground sm:text-sm">
                Amount
              </TableHead>
              <TableHead className="hidden min-w-[190px] text-xs font-medium text-muted-foreground sm:text-sm md:table-cell">
                Accounting
              </TableHead>
              <TableHead className="hidden min-w-[150px] text-xs font-medium text-muted-foreground sm:text-sm lg:table-cell">
                Pricing
              </TableHead>
              <TableHead className="hidden min-w-[150px] text-xs font-medium text-muted-foreground sm:text-sm xl:table-cell">
                Network
              </TableHead>
              <TableHead className="min-w-[100px] text-xs font-medium text-muted-foreground sm:text-sm">
                Status
              </TableHead>
              <TableHead className="w-[40px]"></TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {isRefreshing ? (
              Array.from({ length: Math.min(pageSize, 10) }).map((_, index) => (
                <TableRow key={`refresh-${index}`}>
                  <TableCell>
                    <div className="space-y-2">
                      <Skeleton className="h-4 w-48 max-w-full" />
                      <Skeleton className="h-3 w-72 max-w-full" />
                    </div>
                  </TableCell>
                  <TableCell className="text-right">
                    <div className="space-y-2">
                      <Skeleton className="ml-auto h-4 w-24" />
                      <Skeleton className="ml-auto h-3 w-16" />
                    </div>
                  </TableCell>
                  <TableCell className="hidden md:table-cell">
                    <Skeleton className="h-5 w-28" />
                  </TableCell>
                  <TableCell className="hidden lg:table-cell">
                    <Skeleton className="h-5 w-24" />
                  </TableCell>
                  <TableCell className="hidden xl:table-cell">
                    <Skeleton className="h-5 w-20" />
                  </TableCell>
                  <TableCell>
                    <Skeleton className="h-6 w-24" />
                  </TableCell>
                  <TableCell>
                    <Skeleton className="size-8 rounded-md" />
                  </TableCell>
                </TableRow>
              ))
            ) : paginatedTransactions.length === 0 ? (
              <TableRow>
                <TableCell
                  colSpan={7}
                  className="h-24 text-center text-sm text-muted-foreground"
                >
                  No transactions found matching your filters.
                </TableCell>
              </TableRow>
            ) : (
              paginatedTransactions.map((txn) => {
                const draft = getDraft(txn);
                const rowTaxClassification = austrianTaxClassificationFor(
                  draft.atRegime,
                  draft.atCategory,
                );
                const rowPricingValue = pricingSelectionValue(
                  draft.pricingSourceKind,
                  draft.pricingQuality,
                );
                const StatusIcon = transactionStatusIcons[draft.reviewStatus];
                const explorer = explorerForTransaction(txn, explorerSettings);
                const flow = displayFlow(txn);
                const showPrimaryLabel = !isRedundantTransactionLabel(
                  draft.label,
                  flow,
                );
                const tagPreview = draft.tags;
                const networkLabel =
                  flow === "swap" || flow === "layer-transition"
                    ? pairRailLabel(txn)
                    : txn.paymentMethod;
                const amountBtc = transactionBtc(txn);
                const signedAmountBtc =
                  flow === "outgoing" ? -amountBtc : amountBtc;
                const signedAmountEur =
                  txn.amount === null
                    ? null
                    : flow === "outgoing"
                      ? -txn.amount
                      : txn.amount;
                const primaryAmount =
                  flow === "incoming" || flow === "outgoing"
                    ? formatSignedDisplayMoney(
                        signedAmountEur,
                        signedAmountBtc,
                        currency,
                      )
                    : formatDisplayMoney(txn.amount, amountBtc, currency);
                const FlowIcon =
                  flow === "incoming"
                    ? ArrowDownRight
                    : flow === "outgoing"
                      ? ArrowUpRight
                      : ArrowLeftRight;
                const amountTone =
                  flow === "incoming"
                    ? "text-emerald-700 dark:text-emerald-300"
                    : flow === "outgoing"
                      ? "text-red-700 dark:text-red-300"
                      : "text-muted-foreground";
                return (
                  <TableRow
                    key={txn.id}
                    className="cursor-pointer align-top hover:bg-muted/35"
                    onClick={() => openTransactionDetail(txn)}
                  >
                    <TableCell className="min-w-[280px]">
                      <div className="flex min-w-0 items-start gap-3">
                        <span
                          className={cn(
                            "mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border",
                            transactionFlowStyles[flow],
                          )}
                          aria-hidden="true"
                        >
                          <FlowIcon className="size-4" />
                        </span>
                        <div className="min-w-0">
                          <div className="flex min-w-0 flex-wrap items-center gap-1.5">
                            <span
                              className={cn(
                                "truncate text-sm font-medium text-foreground",
                                blurClass(hideSensitive),
                              )}
                            >
                              {txn.counterparty}
                            </span>
                            {showPrimaryLabel ? (
                              <Badge variant="secondary" className="rounded-md">
                                {draft.label}
                              </Badge>
                            ) : null}
                          </div>
                          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-[10px] text-muted-foreground sm:text-xs">
                            <span
                              className={cn("truncate", blurClass(hideSensitive))}
                            >
                              {txn.wallet || txn.paymentMethod}
                            </span>
                            <span aria-hidden="true">·</span>
                            <span>{txn.date}</span>
                            <span aria-hidden="true">·</span>
                            {explorer ? (
                              <button
                                type="button"
                                className={cn(
                                  "inline-flex max-w-[20ch] items-center gap-1 truncate font-mono text-left underline-offset-4 hover:underline",
                                  blurClass(hideSensitive),
                                )}
                                title={`Open ${txn.txnId} on ${explorer.label}`}
                                onClick={(event) => {
                                  event.stopPropagation();
                                  setExplorerTransaction(txn);
                                }}
                              >
                                <span className="truncate">
                                  {formatShortTxid(txn.txnId)}
                                </span>
                                <ExternalLink
                                  className="size-3 shrink-0 text-muted-foreground"
                                  aria-hidden="true"
                                />
                              </button>
                            ) : (
                              <span
                                className={cn(
                                  "truncate font-mono",
                                  blurClass(hideSensitive),
                                )}
                              >
                                {formatShortTxid(txn.txnId)}
                              </span>
                            )}
                          </div>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell className="min-w-[140px] text-right">
                      <CurrencyToggleText
                        className={cn(
                          "text-sm font-semibold tabular-nums",
                          amountTone,
                          blurClass(hideSensitive),
                        )}
                      >
                        {primaryAmount}
                      </CurrencyToggleText>
                      <div
                        className={cn(
                          "mt-1 text-[10px] text-muted-foreground tabular-nums sm:text-xs",
                          blurClass(hideSensitive),
                        )}
                      >
                        {formatCounterDisplayMoney(
                          txn.amount,
                          amountBtc,
                          currency,
                        )}
                      </div>
                    </TableCell>
                    <TableCell className="hidden md:table-cell">
                      <div className="flex max-w-[210px] flex-wrap gap-1">
                        {tagPreview.slice(0, 2).map((tag) => (
                          <Badge
                            key={tag}
                            variant="outline"
                            className={cn("rounded-md", blurClass(hideSensitive))}
                          >
                            {tag}
                          </Badge>
                        ))}
                        {tagPreview.length > 2 && (
                          <Badge variant="outline" className="rounded-md">
                            +{tagPreview.length - 2}
                          </Badge>
                        )}
                      </div>
                      <p className="mt-1 truncate text-[10px] text-muted-foreground sm:text-xs">
                        {rowTaxClassification.shortLabel}
                      </p>
                    </TableCell>
                    <TableCell className="hidden lg:table-cell">
                      <span
                        className={cn(
                          "inline-flex items-center rounded-md px-2 py-1 text-[10px] font-medium sm:text-xs",
                          pricingSourceStyles[rowPricingValue],
                        )}
                      >
                        {pricingSourceLabel(
                          draft.pricingSourceKind,
                          draft.pricingQuality,
                        )}
                      </span>
                      <p
                        className={cn(
                          "mt-1 truncate text-[10px] text-muted-foreground sm:text-xs",
                          blurClass(hideSensitive),
                        )}
                      >
                        {draft.pricingSourceKind === "manual_override"
                          ? `${draft.manualCurrency} ${draft.manualValue || "value pending"}`
                          : txn.rate
                            ? `${currencyFormatter.format(txn.rate)} / BTC`
                            : "Awaiting price"}
                      </p>
                    </TableCell>
                    <TableCell className="hidden xl:table-cell">
                      <div className="flex flex-wrap gap-1">
                        <span className="inline-flex items-center rounded-md border px-2 py-0.5 text-[10px] font-normal text-muted-foreground sm:text-xs">
                          {networkLabel}
                        </span>
                      </div>
                    </TableCell>
                    <TableCell className="min-w-[120px]">
                      <span
                        className={cn(
                          "inline-flex items-center gap-1 rounded-md px-2 py-1 text-[10px] font-medium sm:text-xs",
                          transactionStatusStyles[draft.reviewStatus],
                        )}
                      >
                        <StatusIcon className="size-3" aria-hidden="true" />
                        {transactionStatusLabels[draft.reviewStatus]}
                      </span>
                      <p className="mt-1 hidden text-[10px] text-muted-foreground sm:block sm:text-xs">
                        {draft.excluded
                          ? "Excluded"
                          : draft.taxable
                            ? "Taxable"
                            : "Not taxable"}
                      </p>
                    </TableCell>
                    <TableCell>
                      <DropdownMenu>
                        <DropdownMenuTrigger asChild>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="size-7 text-muted-foreground hover:text-foreground sm:size-8"
                            aria-label={`Open actions for ${txn.txnId}`}
                            onClick={(event) => event.stopPropagation()}
                          >
                            <MoreHorizontal className="size-3.5 sm:size-4" />
                          </Button>
                        </DropdownMenuTrigger>
                        <DropdownMenuContent align="end">
                          <DropdownMenuItem onSelect={() => openTransactionDetail(txn)}>
                            <Eye className="mr-2 size-4" aria-hidden="true" />
                            View Details
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onSelect={() => openTransactionDetail(txn, "classify")}
                          >
                            <Pencil
                              className="mr-2 size-4"
                              aria-hidden="true"
                            />
                            Classify
                          </DropdownMenuItem>
                          <DropdownMenuItem
                            onSelect={() => copyText(txn.explorerId ?? txn.txnId)}
                          >
                            <Copy className="mr-2 size-4" aria-hidden="true" />
                            Copy ID
                          </DropdownMenuItem>
                          <DropdownMenuSeparator />
                          <DropdownMenuItem
                            className="text-destructive"
                            onSelect={(event: Event) => {
                              event.preventDefault();
                              if (typeof window === "undefined") return;
                              window.confirm(
                                "Void this transaction? This cannot be undone.",
                              );
                            }}
                          >
                            <X className="mr-2 size-4" aria-hidden="true" />
                            Exclude Transaction
                          </DropdownMenuItem>
                        </DropdownMenuContent>
                      </DropdownMenu>
                    </TableCell>
                  </TableRow>
                );
              })
            )}
          </TableBody>
        </Table>
      </div>

      <div className="flex flex-col items-center justify-between gap-3 border-t px-3 py-3 sm:flex-row sm:px-6">
        <div className="flex items-center gap-2 text-xs text-muted-foreground sm:text-sm">
          <span className="hidden sm:inline">Rows per page:</span>
          <Select
            value={pageSize.toString()}
            onValueChange={(value: string) => setPageSize(Number(value))}
          >
            <SelectTrigger className="h-8 w-[70px]">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {PAGE_SIZE_OPTIONS.map((size) => (
                <SelectItem key={size} value={size.toString()}>
                  {size}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <span className="text-muted-foreground">
            {isRefreshing
              ? "Refreshing"
              : filteredTransactions.length === 0
              ? "0"
              : `${(currentPage - 1) * pageSize + 1}-${Math.min(
                  currentPage * pageSize,
                  filteredTransactions.length,
                )}`}{" "}
            {isRefreshing ? "" : `of ${filteredTransactions.length}`}
          </span>
        </div>

        <div className="flex items-center gap-1">
          <Button
            variant="outline"
            size="icon"
            className="size-8"
            onClick={() => goToPage(1)}
            disabled={currentPage === 1}
            aria-label="Go to first page"
          >
            <ChevronsLeft className="size-4" />
          </Button>
          <Button
            variant="outline"
            size="icon"
            className="size-8"
            onClick={() => goToPage(currentPage - 1)}
            disabled={currentPage === 1}
            aria-label="Go to previous page"
          >
            <ChevronLeft className="size-4" />
          </Button>

          <div className="flex items-center gap-1 px-2">
            {Array.from({ length: Math.min(totalPages, 5) }, (_, i) => {
              let pageNum: number;
              if (totalPages <= 5) {
                pageNum = i + 1;
              } else if (currentPage <= 3) {
                pageNum = i + 1;
              } else if (currentPage >= totalPages - 2) {
                pageNum = totalPages - 4 + i;
              } else {
                pageNum = currentPage - 2 + i;
              }

              return (
                <Button
                  key={pageNum}
                  variant={currentPage === pageNum ? "default" : "ghost"}
                  size="icon"
                  className="size-8"
                  onClick={() => goToPage(pageNum)}
                >
                  {pageNum}
                </Button>
              );
            })}
          </div>

          <Button
            variant="outline"
            size="icon"
            className="size-8"
            onClick={() => goToPage(currentPage + 1)}
            disabled={currentPage === totalPages || totalPages === 0}
            aria-label="Go to next page"
          >
            <ChevronRight className="size-4" />
          </Button>
          <Button
            variant="outline"
            size="icon"
            className="size-8"
            onClick={() => goToPage(totalPages)}
            disabled={currentPage === totalPages || totalPages === 0}
            aria-label="Go to last page"
          >
            <ChevronsRight className="size-4" />
          </Button>
        </div>
      </div>
      </div>
      <ExplorerOpenDialog
        transaction={explorerTransaction}
        target={explorerTarget}
        onTransactionChange={setExplorerTransaction}
      />
      <TransactionDetailController
        transaction={detailTransaction}
        initialTab={detailInitialTab}
        hideSensitive={hideSensitive}
        currency={currency}
        explorerSettings={explorerSettings}
        nowRate={nowRate}
        navList={filteredTransactions}
        onOpenChange={(open) => {
          if (!open) {
            setDetailTransaction(null);
            updateTransactionDetailParams(null);
          }
        }}
        onNavigate={(txn, tab) => openTransactionDetail(txn, tab)}
      />
    </>
  );
};

export { TransactionsTable };
