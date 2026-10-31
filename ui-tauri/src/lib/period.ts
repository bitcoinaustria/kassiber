// Shared vocabulary for the chart/table time ranges used by the Overview
// treasury chart and the Transactions workbench. Both screens persist the same
// key in `bookChartPeriods`, so the key list, the URL codec, and the
// history-based gating have to agree — they used to be copied per screen and
// drifted. A key is either a range (30 days up to the whole history) or one
// calendar year of the book. Date arithmetic stays per screen: the treasury
// chart windows in UTC against the book's latest data point, the table in local
// days against today, a year included. Exports keep the book's own calendar.

/** A window counted back from the end of the data. */
export type RangePeriodKey = "30days" | "1year" | "4years" | "10years" | "all";
/** One calendar year, as its number ("2033"). */
export type YearPeriodKey = `${number}`;
export type ResolvedPeriodKey = RangePeriodKey | YearPeriodKey;
export type PeriodKey = "auto" | ResolvedPeriodKey;

/** A year the history holds, newest first, with what it holds. */
export type PeriodYear = { year: number; count: number };

// Lengths grow roughly by an order of magnitude; 4 years is a halving cycle.
export const RANGE_PERIOD_KEYS: RangePeriodKey[] = [
  "30days",
  "1year",
  "4years",
  "10years",
  "all",
];

const LONG_RANGES = [
  { key: "4years", years: 4 },
  { key: "10years", years: 10 },
] as const satisfies ReadonlyArray<{ key: RangePeriodKey; years: number }>;

const MS_PER_YEAR = 365.2425 * 24 * 60 * 60 * 1000;

export function isYearPeriod(period: string): period is YearPeriodKey {
  return /^\d{4}$/.test(period);
}

export function yearPeriod(year: number): YearPeriodKey {
  return `${year}`;
}

export function isPeriodKey(value: unknown): value is PeriodKey {
  return (
    typeof value === "string" &&
    (value === "auto" ||
      (RANGE_PERIOD_KEYS as string[]).includes(value) ||
      isYearPeriod(value))
  );
}

// Typed short forms. Ranges the picker no longer offers still arrive from
// saved settings and old links, and land on the closest one it does.
const SHORT_FORMS: Record<string, RangePeriodKey> = {
  "30d": "30days",
  "3m": "1year",
  "6m": "1year",
  "12m": "1year",
  "1y": "1year",
  "4y": "4years",
  "5y": "4years",
  "10y": "10years",
  "15y": "10years",
};

// `?period=` accepts what a human would type: case, separators, the usual
// short forms (30d, 6mo, 4yrs, max), and a bare year.
export function normalizePeriodParam(
  value: string | null,
  now = new Date(),
): PeriodKey | null {
  if (!value) return null;
  const normalized = value.toLowerCase().replace(/[\s_-]/g, "");
  if (normalized === "auto" || normalized === "automatic") return "auto";
  if (normalized === "all" || normalized === "max") return "all";
  if (normalized === "ytd") return yearPeriod(now.getFullYear());
  if (isYearPeriod(normalized)) return normalized;
  const shortForm = /^(\d+)(d|day|days|m|mo|mos|month|months|y|yr|yrs|year|years)$/.exec(
    normalized,
  );
  if (!shortForm) return null;
  return SHORT_FORMS[`${Number(shortForm[1])}${shortForm[2][0]}`] ?? null;
}

export function periodParamFromUrl(fallback: PeriodKey): PeriodKey {
  if (typeof window === "undefined") return fallback;
  const params = new URLSearchParams(window.location.search);
  return normalizePeriodParam(params.get("period")) ?? fallback;
}

export function historyYearsBetween(earliest: Date, latest: Date): number {
  return Math.max(0, (latest.valueOf() - earliest.valueOf()) / MS_PER_YEAR);
}

// Ranges a user can pick: never offer a window longer than the book's history.
export function selectablePeriods(historyYears: number): RangePeriodKey[] {
  return [
    "30days",
    "1year",
    ...LONG_RANGES.filter((period) => historyYears >= period.years).map(
      (period) => period.key,
    ),
    "all",
  ];
}

// Windows "auto" may resolve to. Long windows unlock a little earlier than they
// become selectable (80% of their span) so auto can settle on a bounded window
// instead of jumping straight to "all" for a nearly-that-old book.
export function autoCandidatePeriods(historyYears: number): RangePeriodKey[] {
  return [
    "1year",
    ...LONG_RANGES.filter(
      (period) => historyYears >= period.years * 0.8,
    ).map((period) => period.key),
    "all",
  ];
}
