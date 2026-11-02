/**
 * How the tax year's result is built, in the form the report states it. An
 * Austrian book lists its E 1kv and E 1 fields with what stays off the return;
 * a generic book lists its holding periods and income. Rows link to their
 * transactions instead of repeating them: the rows themselves, and how each
 * gain was calculated, live in Transactions.
 */

import { Fragment } from "react";
import { Link } from "@tanstack/react-router";
import { ArrowUpRight } from "lucide-react";

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { formatFiatAmount } from "@/lib/currency";
import { formatCount } from "@/lib/localeFormat";
import { cn } from "@/lib/utils";

import { InfoNote } from "./InfoNote";
import {
  signedFiat,
  transactionsHref,
  type ReportBreakdownGroup,
  type ReportBreakdownRow,
  type ReportRegion,
} from "./reportModel";

const gainClassName = (value: number) =>
  value > 0
    ? "text-emerald-700 dark:text-emerald-300"
    : value < 0
      ? "text-red-700 dark:text-red-300"
      : "text-muted-foreground";

export function ReportBreakdown({
  region,
  groups,
  year,
  fiatCurrency,
  hideSensitive,
}: {
  region: ReportRegion;
  groups: ReportBreakdownGroup[];
  year: number;
  fiatCurrency: string;
  hideSensitive: boolean;
}) {
  const sensitive = hideSensitive ? "sensitive" : "";
  const austrian = region === "AT";
  const columns = austrian ? 4 : 5;

  return (
    <section className="kb-surface overflow-hidden" aria-labelledby="report-breakdown-title">
      <div className="flex items-center justify-between gap-3 border-b px-3 py-2.5 sm:px-4">
        <h2 id="report-breakdown-title" className="m-0 text-sm font-medium">
          {austrian ? "Tax fields" : "Gains and income"}
        </h2>
        <Link
          to={transactionsHref(year)}
          className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
        >
          All of {year} in Transactions
          <ArrowUpRight className="size-3.5" aria-hidden="true" />
        </Link>
      </div>

      {groups.length ? (
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              {austrian ? (
                <>
                  <TableHead className="w-0 pl-3 sm:pl-4">Field</TableHead>
                  <TableHead>Description</TableHead>
                </>
              ) : (
                <TableHead className="pl-3 sm:pl-4">Kind</TableHead>
              )}
              <TableHead className="text-right">Transactions</TableHead>
              {austrian ? null : (
                <>
                  <TableHead className="hidden text-right @2xl/reports:table-cell">
                    Proceeds
                  </TableHead>
                  <TableHead className="hidden text-right @2xl/reports:table-cell">
                    Cost basis
                  </TableHead>
                </>
              )}
              <TableHead className="pr-3 text-right sm:pr-4">
                {austrian ? "Amount" : "Gain / loss"}
              </TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {groups.map((group) => (
              <Fragment key={group.title || "rows"}>
                {group.title ? (
                  <TableRow className="bg-muted/40 hover:bg-muted/40">
                    <th
                      scope="rowgroup"
                      colSpan={columns}
                      className="py-1.5 pl-3 text-left text-xs font-medium text-muted-foreground sm:pl-4"
                    >
                      {group.title}
                    </th>
                  </TableRow>
                ) : null}
                {group.rows.map((row) =>
                  austrian ? (
                    <AustrianRow
                      key={row.key}
                      row={row}
                      reported={group.reported}
                      fiatCurrency={fiatCurrency}
                      sensitive={sensitive}
                    />
                  ) : (
                    <GenericRow
                      key={row.key}
                      row={row}
                      fiatCurrency={fiatCurrency}
                      sensitive={sensitive}
                    />
                  ),
                )}
              </Fragment>
            ))}
          </TableBody>
        </Table>
      ) : (
        <p className="m-0 px-4 py-8 text-center text-sm text-muted-foreground">
          No disposals or income in {year}.
        </p>
      )}
    </section>
  );
}

function AustrianRow({
  row,
  reported,
  fiatCurrency,
  sensitive,
}: {
  row: ReportBreakdownRow;
  reported: boolean;
  fiatCurrency: string;
  sensitive: string;
}) {
  const empty =
    row.transactionCount === 0 && Math.abs(row.amount ?? 0) < 0.005;
  return (
    <TableRow
      className={cn(
        row.attention && !empty
          ? "text-amber-700 dark:text-amber-400"
          : (empty || !reported) && "text-muted-foreground",
      )}
    >
      <TableCell className="pl-3 font-medium tabular-nums whitespace-nowrap sm:pl-4">
        {row.code ?? ""}
      </TableCell>
      <TableCell className="w-full max-w-0">
        <span className="flex min-w-0 items-center gap-1.5">
          <span className="truncate">{row.label}</span>
          {row.note ? <InfoNote label={`About ${row.label}`} notes={[row.note]} /> : null}
        </span>
      </TableCell>
      <TableCell className="text-right">
        <TransactionsLink row={row} />
      </TableCell>
      <TableCell
        className={cn(
          "pr-3 text-right tabular-nums whitespace-nowrap sm:pr-4",
          reported && "font-medium",
          sensitive,
        )}
      >
        {row.amount === null ? "—" : formatFiatAmount(row.amount, fiatCurrency)}
      </TableCell>
    </TableRow>
  );
}

function GenericRow({
  row,
  fiatCurrency,
  sensitive,
}: {
  row: ReportBreakdownRow;
  fiatCurrency: string;
  sensitive: string;
}) {
  const income = row.key === "income";
  const gain = row.amount ?? 0;
  return (
    <TableRow>
      <TableCell className="pl-3 sm:pl-4">{row.label}</TableCell>
      <TableCell className="text-right">
        <TransactionsLink row={row} />
      </TableCell>
      <TableCell
        className={cn(
          "hidden text-right text-muted-foreground tabular-nums @2xl/reports:table-cell",
          sensitive,
        )}
      >
        {income ? "—" : formatFiatAmount(row.proceeds, fiatCurrency)}
      </TableCell>
      <TableCell
        className={cn(
          "hidden text-right text-muted-foreground tabular-nums @2xl/reports:table-cell",
          sensitive,
        )}
      >
        {income ? "—" : formatFiatAmount(row.cost, fiatCurrency)}
      </TableCell>
      <TableCell
        className={cn(
          "pr-3 text-right font-medium tabular-nums whitespace-nowrap sm:pr-4",
          gainClassName(gain),
          sensitive,
        )}
      >
        {signedFiat(gain, fiatCurrency)}
      </TableCell>
    </TableRow>
  );
}

function TransactionsLink({ row }: { row: ReportBreakdownRow }) {
  const count = formatCount(row.transactionCount);
  if (!row.href) {
    return <span className="text-muted-foreground tabular-nums">{count}</span>;
  }
  const label = row.exact
    ? `Show these ${count} in Transactions`
    : `Open the year in Transactions; ${count} is too many to list one by one`;
  return (
    <Link
      to={row.href}
      title={label}
      aria-label={`${row.code ? `${row.code} ` : ""}${row.label}: ${label}`}
      className="inline-flex items-center gap-0.5 rounded-sm tabular-nums underline-offset-4 hover:underline"
    >
      {count}
      <ArrowUpRight className="size-3 text-muted-foreground" aria-hidden="true" />
    </Link>
  );
}
