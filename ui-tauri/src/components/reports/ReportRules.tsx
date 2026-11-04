/**
 * The rules the report was computed under, read from the book: its region,
 * its gains algorithm and, for generic books, the holding period. Changing
 * them is a book setting that reprocesses journals, not a report option, so
 * they stay in view as a short card beside the files rather than a tab.
 */

import { Link } from "@tanstack/react-router";
import { ArrowUpRight } from "lucide-react";

import { formatCount } from "@/lib/localeFormat";

import {
  formatTaxRate,
  methodLabel,
  type ReportRegion,
} from "./reportModel";

export function ReportRules({
  region,
  method,
  longTermDays,
  taxRate,
  e1kv = true,
}: {
  region: ReportRegion;
  method: string;
  /** Generic books' holding period; `null` from an older daemon. */
  longTermDays: number | null;
  /** The core's rate for the indicative tax, when it gives one. */
  taxRate?: string;
  /** `false` for an Austrian year before the crypto Kennzahlen. */
  e1kv?: boolean;
}) {
  const rate = formatTaxRate(taxRate);
  const facts: Array<[string, string]> =
    region === "AT"
      ? [
          ["Region", "Austria"],
          ["Method", methodLabel(method)],
          // The rate is the core's; without one (a non-EUR book) none is shown.
          [
            "Tax",
            e1kv
              ? `E 1kv${rate ? ` ${rate}` : ""} · KZ 801 income-tax rate`
              : "KZ 801 income-tax rate · no crypto Kennzahlen before 2022",
          ],
          ["Altbestand", "Bought before 1 Mar 2021 · tax-free after a year"],
          ["Tax year", "Calendar year, Vienna time"],
        ]
      : [
          ["Region", "Generic"],
          ["Method", methodLabel(method)],
          ...(longTermDays === null
            ? []
            : ([
                [
                  "Long-term after",
                  `${formatCount(longTermDays)} day${longTermDays === 1 ? "" : "s"}`,
                ],
              ] as Array<[string, string]>)),
          ["Tax", "No rate applied"],
          ["Tax year", "Calendar year, UTC"],
        ];

  return (
    <section className="kb-surface overflow-hidden" aria-labelledby="report-rules-title">
      <div className="flex items-center justify-between gap-3 border-b px-3 py-2.5 sm:px-4">
        <h2 id="report-rules-title" className="m-0 text-sm font-medium">
          Rules
        </h2>
        <Link
          to="/books"
          className="inline-flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
        >
          Change in Books
          <ArrowUpRight className="size-3.5" aria-hidden="true" />
        </Link>
      </div>
      <dl className="m-0 grid grid-cols-[5.5rem_minmax(0,1fr)] gap-x-3 gap-y-2 px-3 py-3 text-xs sm:px-4">
        {facts.map(([label, value]) => (
          <div key={label} className="contents">
            <dt className="text-muted-foreground">{label}</dt>
            <dd className="m-0">{value}</dd>
          </div>
        ))}
      </dl>
    </section>
  );
}
