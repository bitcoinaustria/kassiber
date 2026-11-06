/**
 * The wallet's own transactions as a dense table: one line per row, its
 * amount in BTC and its value when booked side by side. Rows open in the
 * docked transaction detail; the full, filterable list stays on the
 * Transactions page, which the footer opens scoped to this wallet.
 */

import { Link } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";
import { useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

import {
  blurClass,
  overviewFlowLabelKeys,
  overviewFlowStyles,
  statusLabelKeys,
  statusStyles,
  transactionBtc,
  type Transaction,
} from "@/components/overview-dashboard/model";
import { Button } from "@/components/ui/button";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  formatBtc,
  formatFiatAmount,
  MISSING_FIAT_LABEL,
} from "@/lib/currency";
import { formatCount } from "@/lib/localeFormat";
import { transactionTypeLabel } from "@/lib/transactionTypeLabel";
import { cn } from "@/lib/utils";

const PAGE_SIZE = 25;

const chipClassName =
  "inline-flex items-center rounded-md px-1.5 py-0.5 text-2xs font-medium whitespace-nowrap";

export function WalletTransactionsTable({
  walletLabel,
  transactions,
  totalCount,
  hideSensitive,
  priceEur,
  fiatCurrency,
  onOpenTransaction,
}: {
  walletLabel: string;
  transactions: Transaction[];
  /** Every transaction in the wallet; `transactions` may be the latest slice. */
  totalCount: number;
  hideSensitive: boolean;
  priceEur: number;
  fiatCurrency: string;
  onOpenTransaction: (transaction: Transaction) => void;
}) {
  const { t } = useTranslation("connections");
  const { t: tOverview } = useTranslation("overview");
  const { t: tTransactions } = useTranslation("transactions");
  const [visibleCount, setVisibleCount] = useState(PAGE_SIZE);
  const visible = transactions.slice(0, visibleCount);
  const remaining = transactions.length - visible.length;

  const openOnKey = (event: KeyboardEvent<HTMLElement>, tx: Transaction) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    onOpenTransaction(tx);
  };

  return (
    <div className="kb-surface overflow-hidden">
      <Table>
        <TableHeader>
          <TableRow className="hover:bg-transparent">
            <TableHead className="pl-3 sm:pl-4">
              {t("detail.transactionsTable.date")}
            </TableHead>
            <TableHead className="hidden @2xl/wallet:table-cell">
              {t("detail.transactionsTable.type")}
            </TableHead>
            <TableHead>{t("detail.transactionsTable.description")}</TableHead>
            <TableHead className="text-right">
              {t("detail.transactionsTable.amount")}
            </TableHead>
            <TableHead className="hidden text-right @3xl/wallet:table-cell">
              {t("detail.transactionsTable.value")}
            </TableHead>
            <TableHead className="hidden pr-3 @xl/wallet:table-cell sm:pr-4">
              {t("detail.transactionsTable.status")}
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {visible.map((tx) => {
            const flow = tx.flow ?? "incoming";
            const amountBtc = transactionBtc(tx, priceEur);
            const amountTone =
              flow === "incoming"
                ? "text-emerald-700 dark:text-emerald-300"
                : flow === "outgoing"
                  ? "text-red-700 dark:text-red-300"
                  : "text-muted-foreground";
            const typeLabel = tx.tags[0]
              ? transactionTypeLabel(tTransactions, tx.tags[0])
              : tOverview(overviewFlowLabelKeys[flow]);
            return (
              <TableRow
                key={tx.id}
                role="button"
                tabIndex={0}
                className="cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
                onClick={() => onOpenTransaction(tx)}
                onKeyDown={(event) => openOnKey(event, tx)}
              >
                <TableCell className="pl-3 text-xs text-muted-foreground tabular-nums sm:pl-4">
                  {tx.date}
                </TableCell>
                <TableCell className="hidden @2xl/wallet:table-cell">
                  <span className={cn(chipClassName, overviewFlowStyles[flow])}>
                    {typeLabel}
                  </span>
                </TableCell>
                <TableCell className="max-w-0 w-full">
                  <span
                    className={cn("block truncate", blurClass(hideSensitive))}
                  >
                    {tx.counterparty}
                  </span>
                </TableCell>
                <TableCell
                  className={cn(
                    "text-right font-medium tabular-nums",
                    amountTone,
                    blurClass(hideSensitive),
                  )}
                >
                  {formatBtc(amountBtc, { sign: amountBtc !== 0 })}
                </TableCell>
                <TableCell
                  className={cn(
                    "hidden text-right text-xs text-muted-foreground tabular-nums @3xl/wallet:table-cell",
                    blurClass(hideSensitive),
                  )}
                >
                  {tx.amount === null
                    ? MISSING_FIAT_LABEL
                    : formatFiatAmount(
                        Math.abs(tx.amount),
                        tx.fiatCurrency ?? fiatCurrency,
                      )}
                </TableCell>
                <TableCell className="hidden pr-3 @xl/wallet:table-cell sm:pr-4">
                  <span className={cn(chipClassName, statusStyles[tx.status])}>
                    {tOverview(statusLabelKeys[tx.status])}
                  </span>
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
      <div className="flex flex-wrap items-center justify-between gap-2 border-t px-3 py-2 text-xs text-muted-foreground sm:px-4">
        <span className="tabular-nums">
          {t("detail.transactionsTable.showing", {
            shown: formatCount(visible.length),
            total: formatCount(totalCount),
          })}
        </span>
        <div className="flex items-center gap-1">
          {remaining > 0 ? (
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 px-2 text-xs"
              onClick={() => setVisibleCount((count) => count + PAGE_SIZE)}
            >
              {t("detail.transactionsTable.showMore", {
                count: Math.min(PAGE_SIZE, remaining),
              })}
            </Button>
          ) : null}
          <Button
            asChild
            variant="ghost"
            size="sm"
            className="h-7 gap-1 px-2 text-xs"
          >
            <Link
              to="/transactions"
              search={{ wallet: walletLabel }}
              hash="transactions-table"
            >
              {t("detail.transactionsTable.openAll")}
              <ArrowRight className="size-3.5" aria-hidden="true" />
            </Link>
          </Button>
        </div>
      </div>
    </div>
  );
}
