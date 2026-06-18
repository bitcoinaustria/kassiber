import { Link } from "@tanstack/react-router";
import {
  ArrowDownRight,
  ArrowLeftRight,
  ArrowUpRight,
  ChevronLeft,
  ChevronRight,
  Filter,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  formatBtc,
  formatFiatAmount,
  MISSING_FIAT_LABEL,
  type Currency,
} from "@/lib/currency";
import { cn } from "@/lib/utils";

import {
  blurClass,
  formatSignedDisplayMoney,
  overviewFlowLabelKeys,
  overviewFlowStyles,
  statusLabelKeys,
  statusStyles,
  transactionBtc,
  transactionDetailHref,
  transactionStatuses,
  type Transaction,
  type TransactionStatus,
} from "./model";

export const RecentTransactionsTable = ({
  className,
  title,
  transactions,
  hideSensitive,
  currency,
  priceEur,
  fiatCurrency = "EUR",
  showAllLabel,
  showAllTo = "/transactions",
  onOpenTransaction,
}: {
  className?: string;
  title?: string;
  transactions: Transaction[];
  hideSensitive: boolean;
  currency: Currency;
  priceEur: number;
  fiatCurrency?: string;
  showAllLabel?: string;
  showAllTo?: "/transactions" | null;
  onOpenTransaction?: (transaction: Transaction) => void;
}) => {
  const { t } = useTranslation("overview");
  const resolvedTitle = title ?? t("recentTx.title");
  const resolvedShowAllLabel = showAllLabel ?? t("recentTx.showAll");
  const [statusFilter, setStatusFilter] = React.useState<
    TransactionStatus | "all"
  >("all");
  const [currentPage, setCurrentPage] = React.useState(1);
  const [isHydrated, setIsHydrated] = React.useState(false);
  const pageSize = 6;

  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    const nextStatus = params.get("status");
    if (
      nextStatus &&
      (nextStatus === "all" ||
        transactionStatuses.includes(nextStatus as TransactionStatus))
    ) {
      setStatusFilter(nextStatus as TransactionStatus | "all");
    }
    const nextPage = Number(params.get("page"));
    if (!Number.isNaN(nextPage) && nextPage > 0) {
      setCurrentPage(nextPage);
    }
    setIsHydrated(true);
  }, []);

  const filteredTransactions = React.useMemo(() => {
    if (statusFilter === "all") return transactions;
    return transactions.filter((t) => t.status === statusFilter);
  }, [statusFilter, transactions]);

  const totalPages = Math.max(
    1,
    Math.ceil(filteredTransactions.length / pageSize),
  );

  const paginatedTransactions = React.useMemo(() => {
    const startIndex = (currentPage - 1) * pageSize;
    return filteredTransactions.slice(startIndex, startIndex + pageSize);
  }, [filteredTransactions, currentPage, pageSize]);

  React.useEffect(() => {
    setCurrentPage(1);
  }, [statusFilter]);

  React.useEffect(() => {
    if (!isHydrated || typeof window === "undefined") return;
    const params = new URLSearchParams(window.location.search);
    if (statusFilter !== "all") {
      params.set("status", statusFilter);
    } else {
      params.delete("status");
    }
    if (currentPage > 1) {
      params.set("page", String(currentPage));
    } else {
      params.delete("page");
    }
    const nextQuery = params.toString();
    const nextUrl = nextQuery
      ? `${window.location.pathname}?${nextQuery}`
      : window.location.pathname;
    window.history.replaceState(null, "", nextUrl);
  }, [statusFilter, currentPage, isHydrated]);

  const goToPage = (page: number) => {
    setCurrentPage(Math.max(1, Math.min(page, totalPages)));
  };

  const startRow = filteredTransactions.length
    ? (currentPage - 1) * pageSize + 1
    : 0;
  const endRow = Math.min(currentPage * pageSize, filteredTransactions.length);

  return (
    <>
      <div className={cn("rounded-xl border bg-card", className)}>
      <div className="flex items-center justify-between gap-3 px-3 pt-3 sm:px-4">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium">
            {resolvedTitle}
          </span>
          <span className="ml-1 inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-[10px] font-medium text-gray-600 ring-1 ring-gray-500/10 ring-inset sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20">
            {filteredTransactions.length}
          </span>
        </div>

        <div className="flex items-center gap-2">
          {showAllTo ? (
            <Button asChild variant="ghost" size="sm" className="h-8 sm:h-9">
              <Link to={showAllTo}>{resolvedShowAllLabel}</Link>
            </Button>
          ) : null}
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button
                variant="outline"
                size="sm"
                className="h-8 gap-1.5 sm:h-9 sm:gap-2"
              >
                <Filter className="size-3.5 sm:size-4" aria-hidden="true" />
                <span className="hidden sm:inline">{t("recentTx.filter")}</span>
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-[180px]">
              <DropdownMenuLabel>{t("recentTx.filterByStatus")}</DropdownMenuLabel>
              <DropdownMenuCheckboxItem
                checked={statusFilter === "all"}
                onCheckedChange={() => setStatusFilter("all")}
              >
                {t("recentTx.allStatuses")}
              </DropdownMenuCheckboxItem>
              {transactionStatuses.map((status) => (
                <DropdownMenuCheckboxItem
                  key={status}
                  checked={statusFilter === status}
                  onCheckedChange={() => setStatusFilter(status)}
                >
                  {t(statusLabelKeys[status])}
                </DropdownMenuCheckboxItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      <div className="px-3 pt-2.5 pb-3 sm:px-4">
        {paginatedTransactions.length === 0 ? (
          <div className="flex h-24 items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground">
            {t("recentTx.empty")}
          </div>
        ) : (
          <div className="divide-y rounded-lg border bg-background/50">
            {paginatedTransactions.map((tx) => {
              const flow = tx.flow ?? "incoming";
              const FlowIcon =
                flow === "incoming"
                  ? ArrowDownRight
                  : flow === "outgoing"
                    ? ArrowUpRight
                    : ArrowLeftRight;
              const amountBtc = transactionBtc(tx, priceEur);
              const rowFiatCurrency = tx.fiatCurrency ?? fiatCurrency;
              const primaryAmount =
                currency === "btc"
                  ? formatBtc(amountBtc, { sign: true })
                  : formatSignedDisplayMoney(
                      tx.amount,
                      priceEur,
                      currency,
                      rowFiatCurrency,
                    );
              const secondaryAmount =
                currency === "btc"
                  ? tx.amount === null
                    ? MISSING_FIAT_LABEL
                    : formatFiatAmount(Math.abs(tx.amount), rowFiatCurrency)
                  : formatBtc(amountBtc);
              const amountTone =
                flow === "incoming"
                  ? "text-emerald-700 dark:text-emerald-300"
                  : flow === "outgoing"
                    ? "text-red-700 dark:text-red-300"
                    : "text-muted-foreground";
              const flowLabel = t(overviewFlowLabelKeys[flow]);
              const primaryTag = tx.tags[0] ?? flowLabel;
              const extraTags = Math.max(0, tx.tags.length - 1);
              const rowClassName =
                "group flex min-w-0 items-center gap-3 px-3 py-2.5 text-left transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";
              const rowContent = (
                <>
                  <span
                    className={cn(
                      "flex size-8 shrink-0 items-center justify-center rounded-md",
                      overviewFlowStyles[flow],
                    )}
                    aria-hidden="true"
                  >
                    <FlowIcon className="size-4" />
                  </span>
                  <span className="min-w-0 flex-1">
                    <span
                      className={cn(
                        "block truncate text-sm font-medium text-foreground",
                        blurClass(hideSensitive),
                      )}
                    >
                      {tx.counterparty}
                    </span>
                    <span className="mt-1 flex min-w-0 flex-wrap items-center gap-1.5">
                      <span
                        className={cn(
                          "inline-flex items-center rounded-md px-1.5 py-0.5 text-[10px] font-medium",
                          overviewFlowStyles[flow],
                        )}
                      >
                        {primaryTag}
                      </span>
                      {extraTags > 0 && (
                        <span className="text-[10px] text-muted-foreground">
                          +{extraTags}
                        </span>
                      )}
                      <span
                        className={cn(
                          "inline-flex items-center rounded-md px-1.5 py-0.5 text-[10px] font-medium",
                          statusStyles[tx.status],
                        )}
                      >
                        {t(statusLabelKeys[tx.status])}
                      </span>
                      <span className="truncate text-[10px] text-muted-foreground">
                        {tx.date}
                      </span>
                      {tx.scopeLabel ? (
                        <span
                          className={cn(
                            "truncate text-[10px] font-medium text-muted-foreground",
                            blurClass(hideSensitive),
                          )}
                        >
                          {tx.scopeLabel}
                        </span>
                      ) : null}
                    </span>
                    <span
                      className={cn(
                        "mt-1 hidden truncate font-mono text-[10px] text-muted-foreground sm:block",
                        blurClass(hideSensitive),
                      )}
                    >
                      {flowLabel} · {tx.txid}
                    </span>
                  </span>
                  <span className="ml-auto flex shrink-0 flex-col items-end gap-0.5 pl-2 text-right">
                    <CurrencyToggleText
                      className={cn(
                        "text-sm font-semibold tabular-nums",
                        amountTone,
                        blurClass(hideSensitive),
                      )}
                    >
                      {primaryAmount}
                    </CurrencyToggleText>
                    <span
                      className={cn(
                        "text-[10px] text-muted-foreground tabular-nums",
                        blurClass(hideSensitive),
                      )}
                    >
                      {secondaryAmount}
                    </span>
                  </span>
                </>
              );
              if (onOpenTransaction) {
                return (
                  <button
                    key={tx.id}
                    type="button"
                    className={cn(rowClassName, "w-full")}
                    onClick={() => onOpenTransaction(tx)}
                  >
                    {rowContent}
                  </button>
                );
              }
              return (
                <Link
                  key={tx.id}
                  to={transactionDetailHref(tx.id)}
                  className={rowClassName}
                >
                  {rowContent}
                </Link>
              );
            })}
          </div>
        )}
      </div>

      <div className="flex items-center justify-between border-t px-3 py-2.5 text-[10px] text-muted-foreground sm:px-4 sm:text-xs">
        <span>
          {t("recentTx.rangeOfTotal", {
            start: startRow,
            end: endRow,
            total: filteredTransactions.length,
          })}
        </span>
        <div className="flex items-center gap-1">
          <Button
            variant="outline"
            size="icon"
            className="size-7"
            onClick={() => goToPage(currentPage - 1)}
            disabled={currentPage === 1}
            aria-label={t("recentTx.previousPage")}
          >
            <ChevronLeft className="size-3.5" />
          </Button>
          <Button
            variant="outline"
            size="icon"
            className="size-7"
            onClick={() => goToPage(currentPage + 1)}
            disabled={currentPage === totalPages}
            aria-label={t("recentTx.nextPage")}
          >
            <ChevronRight className="size-3.5" />
          </Button>
        </div>
      </div>
      </div>
    </>
  );
};
