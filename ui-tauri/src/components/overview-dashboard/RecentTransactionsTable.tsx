import { Link } from "@tanstack/react-router";
import {
  ArrowDownRight,
  ArrowLeftRight,
  ArrowUpRight,
} from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { CurrencyToggleText } from "@/components/kb/CurrencyToggleText";
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
  type Transaction,
} from "./model";

const RECENT_TX_REVEAL_STEP = 8;

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
  const [visibleCount, setVisibleCount] = React.useState(RECENT_TX_REVEAL_STEP);

  const visibleTransactions = React.useMemo(
    () => transactions.slice(0, visibleCount),
    [transactions, visibleCount],
  );

  React.useEffect(() => {
    setVisibleCount(RECENT_TX_REVEAL_STEP);
  }, [transactions.length]);

  const revealMoreRows = React.useCallback(() => {
    setVisibleCount((current) =>
      Math.min(transactions.length, current + RECENT_TX_REVEAL_STEP),
    );
  }, [transactions.length]);

  const handleScroll = React.useCallback(
    (event: React.UIEvent<HTMLDivElement>) => {
      if (visibleCount >= transactions.length) return;
      const target = event.currentTarget;
      const distanceFromBottom =
        target.scrollHeight - target.scrollTop - target.clientHeight;
      if (distanceFromBottom < 72) revealMoreRows();
    },
    [transactions.length, revealMoreRows, visibleCount],
  );

  return (
    <>
      <div className={cn("overflow-hidden rounded-lg border bg-card", className)}>
      <div className="flex items-center justify-between gap-3 border-b px-3 py-2.5 sm:px-4">
        <div className="flex items-center gap-2">
          <span className="text-sm font-medium">
            {resolvedTitle}
          </span>
        </div>

        {showAllTo ? (
          <Link
            to={showAllTo}
            className="shrink-0 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {resolvedShowAllLabel}
          </Link>
        ) : null}
      </div>

      <div
        className="max-h-[340px] min-h-0 overflow-auto"
        onScroll={handleScroll}
      >
        {visibleTransactions.length === 0 ? (
          <div className="m-3 flex h-24 items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground sm:m-4">
            {t("recentTx.empty")}
          </div>
        ) : (
          <div className="divide-y">
            {visibleTransactions.map((tx) => {
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
                "group flex min-w-0 items-center gap-3 px-3 py-2 text-left transition-colors hover:bg-muted/45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:px-4";
              const rowContent = (
                <>
                  <span
                    className={cn(
                      "flex size-8 shrink-0 items-center justify-center rounded-md",
                      "border",
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
                      {tx.counterparty || tx.txid}
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
      </div>
    </>
  );
};
