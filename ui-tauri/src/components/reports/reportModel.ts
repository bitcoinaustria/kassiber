/**
 * What the Reports screen reads out of `ui.reports.capital_gains`: which
 * rules the book's report follows, the year's totals and their breakdown, and
 * the core's indicative tax. Every figure comes from the daemon; this module
 * only picks and labels them, so the screen cannot compute a tax result.
 */

import { formatFiatAmount } from "@/lib/currency";
import type {
  CapitalGainsReport,
  CostBasisMethod,
  KennzahlRow,
  ReportBreakdownBucket,
} from "@/mocks/reports";

/** The book regions the core supports; anything else reads as generic. */
export type ReportRegion = "AT" | "GENERIC";

export function reportRegion(code: string | undefined): ReportRegion {
  return String(code ?? "").trim().toUpperCase() === "AT" ? "AT" : "GENERIC";
}

const METHOD_LABELS: Record<CostBasisMethod, string> = {
  fifo: "FIFO · first in, first out",
  lifo: "LIFO · last in, first out",
  hifo: "HIFO · highest cost first",
  lofo: "LOFO · lowest cost first",
  moving_average: "Moving average cost",
  moving_average_at: "Austrian method · FIFO for Altbestand, moving average for Neubestand",
};

/** The book's gains algorithm as the report used it, never substituted. */
export function methodLabel(method: string | undefined) {
  const key = String(method ?? "").toLowerCase() as CostBasisMethod;
  return METHOD_LABELS[key] ?? String(method ?? "—").toUpperCase();
}

export interface ReportTotals {
  rowCount: number;
  proceeds: number;
  cost: number;
  /** Disposal gains plus income, as the daemon totals them. */
  gain: number;
  income: number;
  /** The part of `gain` from tax-free Altbestand, which is not filed. */
  taxFreeGain: number;
}

/** The year's totals over every disposal and income row. */
export function reportTotals(report: CapitalGainsReport): ReportTotals {
  const taxFree = report.breakdown?.find((bucket) => bucket.key === "tax_free");
  return {
    rowCount: report.totals?.rowCount ?? 0,
    proceeds: report.totals?.proceedsEur ?? 0,
    cost: report.totals?.costEur ?? 0,
    gain: report.totals?.gainLossEur ?? 0,
    income: report.totals?.incomeEur ?? 0,
    taxFreeGain: taxFree?.gainLossEur ?? 0,
  };
}

export interface ReportTaxEstimate {
  amount: number | null;
  /** E 1 KZ 801 amounts, taxed at the progressive rate and not estimated. */
  progressiveAmount: number;
  /** The core's caveats for the figure. */
  notes: string[];
}

/** The core's indicative tax, or `null` when the core gives none. */
export function reportTaxEstimate(
  report: CapitalGainsReport,
): ReportTaxEstimate | null {
  const tax = report.indicativeTax;
  if (!tax) return null;
  const progressiveCents = Object.values(
    tax.progressiveRateAmountsEurCents ?? {},
  ).reduce((sum, cents) => sum + cents, 0);
  return {
    amount:
      tax.estimatedTaxEurCents === null ? null : tax.estimatedTaxEurCents / 100,
    progressiveAmount: progressiveCents / 100,
    notes: tax.notes ?? [],
  };
}

/** `27.5 %` from the core's decimal rate string. */
export function formatTaxRate(rate: string | undefined) {
  const value = Number(rate);
  if (!Number.isFinite(value)) return "";
  return `${Number((value * 100).toFixed(2))} %`;
}

/** `1 Jan – 31 Dec 2025`; the report copy stays English. */
export function formatReportPeriod(year: number) {
  return `1 Jan – 31 Dec ${year}`;
}

/** `+ € 1.234,56` / `− € 12,00`, unsigned at zero. */
export function signedFiat(value: number, currency: string) {
  const sign = value > 0 ? "+ " : value < 0 ? "− " : "";
  return `${sign}${formatFiatAmount(Math.abs(value), currency)}`;
}

export const AUSTRIAN_FIELD_COPY: Record<
  string,
  { label: string; form: string; note?: string }
> = {
  "172": { label: "Foreign ongoing crypto income", form: "E 1kv" },
  "174": { label: "Foreign realized crypto gains", form: "E 1kv" },
  "176": { label: "Foreign realized crypto losses", form: "E 1kv" },
  "801": {
    label: "Altbestand speculation gains",
    form: "E 1",
    note: "Taxed at your income-tax rate",
  },
};

/** First tax year with the crypto Kennzahlen; the core decides, this names it. */
export const E1KV_FIRST_TAX_YEAR = 2022;

/** Whether the report's year has the crypto Kennzahlen (Austrian books only). */
export function reportE1kvApplies(report: CapitalGainsReport): boolean {
  return reportRegion(report.jurisdictionCode) === "AT" && report.e1kvApplies !== false;
}

/** Rows the report states in no field, which need a look before filing. */
export function reportUnassignedRows(report: CapitalGainsReport): number {
  return report.breakdown?.find((bucket) => bucket.key === "unassigned")?.rowCount ?? 0;
}

/** The book's filing fields, or the Austrian ones awaiting figures. */
export function reportTaxFieldRows(report: CapitalGainsReport): KennzahlRow[] {
  if (report.kennzahlRows?.length) return report.kennzahlRows;
  if (reportRegion(report.jurisdictionCode) !== "AT") return [];
  const e1kv = reportE1kvApplies(report);
  return Object.entries(AUSTRIAN_FIELD_COPY)
    // Before 2022 only the E 1's KZ 801 is a crypto-related field.
    .filter(([, copy]) => e1kv || copy.form !== "E 1kv")
    .map(([code, copy]) => ({
      code,
      label: copy.label,
      form: copy.form,
      amount: null,
      rowCount: 0,
      source: "pending",
    }));
}

/**
 * Transactions, scoped to the year: exactly the given transactions when there
 * are any, the whole year otherwise.
 */
export function transactionsHref(year: number, transactionIds?: string[] | null) {
  const params = new URLSearchParams({ period: String(year) });
  if (transactionIds?.length) params.set("txids", transactionIds.join(","));
  return `/transactions?${params.toString()}`;
}

export interface ReportBreakdownRow {
  key: string;
  /** `KZ 174` for an Austrian field. */
  code?: string;
  label: string;
  note?: string;
  transactionCount: number;
  /** Transactions, or `null` for a group with none. */
  href: string | null;
  /** Whether `href` names the group's transactions or opens the whole year. */
  exact: boolean;
  /** The field's amount, the gain, or `null` where none applies. */
  amount: number | null;
  /** Off the return but possibly taxable, so shown as needing a look. */
  attention?: boolean;
  proceeds: number;
  cost: number;
  income: number;
}

export interface ReportBreakdownGroup {
  /** Empty for a generic book's single group. */
  title: string;
  /** The group's amounts are not filed. */
  reported: boolean;
  rows: ReportBreakdownRow[];
}

function rowLinks(year: number, bucket: ReportBreakdownBucket | undefined) {
  const count = bucket?.transactionCount ?? 0;
  if (!count) return { transactionCount: 0, href: null, exact: false };
  const ids = bucket?.transactionIds ?? null;
  return {
    transactionCount: count,
    href: transactionsHref(year, ids),
    exact: Boolean(ids?.length),
  };
}

const GENERIC_ROWS: Array<{ key: string; label: string }> = [
  { key: "short_term", label: "Short-term gains" },
  { key: "long_term", label: "Long-term gains" },
  { key: "income", label: "Income" },
];

const AUSTRIAN_EXTRA_ROWS: Array<{ key: string; label: string; note: string }> = [
  {
    key: "tax_free",
    label: "Tax-free Altbestand",
    note: "Bought before 1 March 2021 and held over a year",
  },
  {
    key: "swaps",
    label: "Tax-neutral swaps",
    note: "Basis carried into the asset received",
  },
  {
    key: "unassigned",
    label: "Without a field",
    note: "Rows in no field: those before 1 March 2022, which followed the old rules, and any the tax engine could not assign. Check them before filing: they may be taxable.",
  },
];

/**
 * How the year's result is built, in the form the report states it: the
 * Austrian fields with what stays off the return, or a generic book's
 * holding periods and income. Each row links to its transactions.
 */
export function reportBreakdown(report: CapitalGainsReport): ReportBreakdownGroup[] {
  const year = report.year;
  const buckets = new Map(
    (report.breakdown ?? []).map((bucket) => [bucket.key, bucket]),
  );
  const amounts = (bucket: ReportBreakdownBucket | undefined) => ({
    proceeds: bucket?.proceedsEur ?? 0,
    cost: bucket?.costEur ?? 0,
    income: bucket?.incomeEur ?? 0,
  });

  if (reportRegion(report.jurisdictionCode) !== "AT") {
    const rows = GENERIC_ROWS.flatMap(({ key, label }) => {
      const bucket = buckets.get(key);
      if (!bucket?.rowCount) return [];
      return [
        {
          key,
          label,
          ...rowLinks(year, bucket),
          // Income counts its value, as the year's income total does.
          amount: key === "income" ? bucket.incomeEur : bucket.gainLossEur,
          ...amounts(bucket),
        },
      ];
    });
    return rows.length ? [{ title: "", reported: true, rows }] : [];
  }

  const fieldRows = reportTaxFieldRows(report).map((field) => {
    const copy = AUSTRIAN_FIELD_COPY[field.code];
    const bucket = buckets.get(field.code);
    return {
      key: field.code,
      code: `KZ ${field.code}`,
      label: copy?.label ?? field.label,
      note: copy?.note,
      form: copy?.form ?? field.form ?? "E 1kv",
      ...rowLinks(year, bucket),
      amount: field.amount,
      ...amounts(bucket),
    };
  });
  const groups: ReportBreakdownGroup[] = [];
  const e1kv = fieldRows.filter((row) => row.form === "E 1kv");
  const e1 = fieldRows.filter((row) => row.form !== "E 1kv");
  if (e1kv.length) groups.push({ title: "E 1kv", reported: true, rows: e1kv });
  if (e1.length) {
    groups.push({ title: "E 1 · outside the E 1kv", reported: true, rows: e1 });
  }
  const offReturn = AUSTRIAN_EXTRA_ROWS.flatMap(({ key, label, note }) => {
    const bucket = buckets.get(key);
    if (!bucket?.rowCount) return [];
    return [
      {
        key,
        label,
        note,
        ...rowLinks(year, bucket),
        // Swaps carry basis and realize nothing; tax-free rows show the gain
        // that stays off the return.
        amount: key === "swaps" ? null : bucket.gainLossEur,
        ...amounts(bucket),
        // Unlike the other rows here, these may belong on the return.
        attention: key === "unassigned",
      },
    ];
  });
  if (offReturn.length) {
    groups.push({ title: "Not reported", reported: false, rows: offReturn });
  }
  return groups;
}

export type ReportExportFormatId =
  | "csv"
  | "pdf"
  | "xlsx"
  | "summary_pdf"
  | "audit_package"
  | "year_handoff";

export type AuditPackageScope = "active_profile" | "source_funds_case";

export interface AuditPackageOptions {
  scope: AuditPackageScope;
  /** The saved source-of-funds case, when `scope` is one. */
  caseId: string;
  include: {
    copiedAttachments: boolean;
    urlReferences: boolean;
    journalState: boolean;
    reviewState: boolean;
    editHistory: boolean;
  };
}
