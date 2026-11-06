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
import { transactionTypeLabel } from "@/lib/transactionTypeLabel";
import { cn } from "@/lib/utils";

import {
  blurClass,
  formatSignedDisplayMoney,
  overviewFlowLabelKeys,
  overviewFlowStyles,
  rowsThatFit,
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
  limit,
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
  /**
   * Show only this many, with no scrolling inside the card: the card fills
   * the height it is given and Show all opens the rest. Without it the list
   * scrolls and loads more as it nears its end.
   */
  limit?: number;
}) => {
  const { t } = useTranslation("overview");
  const { t: tTransactions } = useTranslation("transactions");
  const resolvedTitle = title ?? t("recentTx.title");
  const resolvedShowAllLabel = showAllLabel ?? t("recentTx.showAll");
  const [visibleCount, setVisibleCount] = React.useState(RECENT_TX_REVEAL_STEP);

  // With a limit, and while the card takes its height from around it (laid
  // absolutely beside the chart), the rows that fit: whole rows only, so
  // none shows cut off at the bottom. Otherwise all `limit` rows.
  const listRef = React.useRef<HTMLDivElement>(null);
  const [fits, setFits] = React.useState<number | null>(null);
  React.useLayoutEffect(() => {
    const list = listRef.current;
    if (!limit || !list || typeof ResizeObserver === "undefined") return;
    const measure = () => {
      const card = list.parentElement;
      const sized = card ? getComputedStyle(card).position === "absolute" : false;
      // A row below the first carries the divider, so it is the full step.
      // Fractional heights: rounded ones add up to a cut-off last row.
      const rows = list.querySelectorAll<HTMLElement>("[data-recent-row]");
      const rowStep = rows[1]
        ? rows[1].getBoundingClientRect().height
        : rows[0]
          ? rows[0].getBoundingClientRect().height + 1
          : 0;
      setFits(sized ? rowsThatFit(list.getBoundingClientRect().height, rowStep) : null);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(list);
    if (list.parentElement) observer.observe(list.parentElement);
    return () => observer.disconnect();
    // Read again when the list changes: in a sized card new rows do not
    // resize anything the observer watches.
  }, [limit, transactions]);
  const visibleTransactions = React.useMemo(
    () => transactions.slice(0, limit ? Math.min(limit, fits ?? limit) : visibleCount),
    [fits, limit, transactions, visibleCount],
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
      <div className={cn("kb-surface flex flex-col overflow-hidden", className)}>
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
        ref={listRef}
        className={limit ? "min-h-0 flex-1 overflow-hidden" : "max-h-[340px] min-h-0 overflow-auto"}
        onScroll={limit ? undefined : handleScroll}
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
              // A real metadata tag wins over the derived type; both arrive as
              // raw strings, so translate only the ones we recognize.
              const primaryTag = tx.tags[0]
                ? transactionTypeLabel(tTransactions, tx.tags[0])
                : flowLabel;
              const extraTags = Math.max(0, tx.tags.length - 1);
              // The status only when it asks for something, and never twice:
              // a row tagged Review keeps the Review chip and names its
              // direction instead of the tag.
              const statusLabel = t(statusLabelKeys[tx.status]);
              const showStatus = tx.status !== "confirmed";
              const tagIsStatus =
                showStatus &&
                (tx.tags[0]?.toLowerCase() === tx.status ||
                  statusLabel.toLowerCase() === primaryTag.toLowerCase());
              const typeLabel = tagIsStatus ? flowLabel : primaryTag;
              const rowClassName =
                "group flex min-w-0 items-center gap-2.5 px-3 py-1.5 text-left transition-colors hover:bg-muted/45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring sm:px-4";
              const rowContent = (
                <>
                  <span
                    className={cn(
                      "flex size-6 shrink-0 items-center justify-center rounded-md border",
                      overviewFlowStyles[flow],
                    )}
                    aria-hidden="true"
                  >
                    <FlowIcon className="size-3.5" />
                  </span>
                  <span className="min-w-0 flex-1 leading-tight">
                    <span
                      className={cn(
                        "block truncate text-[13px] font-medium text-foreground",
                        blurClass(hideSensitive),
                      )}
                    >
                      {tx.counterparty || tx.txid}
                    </span>
                    <span className="mt-0.5 flex min-w-0 items-center gap-1.5 text-2xs text-muted-foreground">
                      <span className="shrink-0 tabular-nums">{tx.date}</span>
                      <span aria-hidden="true">·</span>
                      <span className="truncate">
                        {extraTags > 0 ? `${typeLabel} +${extraTags}` : typeLabel}
                      </span>
                      {showStatus ? (
                        <span
                          className={cn(
                            // The line's own height, so every row is as tall as the next.
                            "shrink-0 rounded-md px-1 font-medium",
                            statusStyles[tx.status],
                          )}
                        >
                          {statusLabel}
                        </span>
                      ) : null}
                      {tx.scopeLabel ? (
                        <span
                          className={cn(
                            "truncate font-medium",
                            blurClass(hideSensitive),
                          )}
                        >
                          {tx.scopeLabel}
                        </span>
                      ) : null}
                    </span>
                  </span>
                  <span className="ml-auto flex shrink-0 flex-col items-end pl-2 text-right leading-tight">
                    <CurrencyToggleText
                      className={cn(
                        "text-[13px] font-semibold tabular-nums",
                        amountTone,
                        blurClass(hideSensitive),
                      )}
                    >
                      {primaryAmount}
                    </CurrencyToggleText>
                    <span
                      className={cn(
                        "mt-0.5 text-2xs text-muted-foreground tabular-nums",
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
                    data-recent-row
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
                  data-recent-row
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
