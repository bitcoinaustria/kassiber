// Shared vocabulary for the chart/table time ranges used by the Overview
// treasury chart and the Transactions workbench. Both screens persist the same
// key in `bookChartPeriods`, so the key list, the URL codec, and the
// history-based gating have to agree — they used to be copied per screen and
// drifted (the Overview chips had no 10y/15y even though "auto" could resolve
// to them). Date arithmetic stays per screen: the treasury chart windows in UTC
// against the book's latest data point, the table in local days against today.

export type PeriodKey =
  | "auto"
  | "30days"
  | "3months"
  | "6months"
  | "ytd"
  | "1year"
  | "5years"
  | "10years"
  | "15years"
  | "all";

export type ResolvedPeriodKey = Exclude<PeriodKey, "auto">;

export const PERIOD_KEYS: PeriodKey[] = [
  "auto",
  "30days",
  "3months",
  "6months",
  "ytd",
  "1year",
  "5years",
  "10years",
  "15years",
  "all",
];

const BASE_PERIOD_KEYS: ResolvedPeriodKey[] = [
  "30days",
  "3months",
  "6months",
  "ytd",
  "1year",
];

const LONG_PERIOD_YEARS = [
  { key: "5years", years: 5 },
  { key: "10years", years: 10 },
  { key: "15years", years: 15 },
] as const satisfies ReadonlyArray<{ key: ResolvedPeriodKey; years: number }>;

const MS_PER_YEAR = 365.2425 * 24 * 60 * 60 * 1000;

// `?period=` accepts what a human would type: case, separators, and the usual
// short forms (30d, 3mo, 5yrs, max) all fold onto a canonical key.
export function normalizePeriodParam(value: string | null): PeriodKey | null {
  if (!value) return null;
  const normalized = value.toLowerCase().replace(/[\s_-]/g, "");
  if (normalized === "automatic") return "auto";
  if (normalized === "max") return "all";
  const shortForm = /^(\d+)(d|day|days|m|mo|mos|month|months|y|yr|yrs|year|years)$/.exec(
    normalized,
  );
  if (!shortForm) {
    return PERIOD_KEYS.find((key) => key === normalized) ?? null;
  }
  // Keys are inconsistently pluralized ("1year" but "5years"), so accept both.
  const unit = shortForm[2].startsWith("d")
    ? "days"
    : shortForm[2].startsWith("m")
      ? "months"
      : "years";
  const spellings = [
    `${shortForm[1]}${unit}`,
    `${shortForm[1]}${unit.slice(0, -1)}`,
  ];
  return PERIOD_KEYS.find((key) => spellings.includes(key)) ?? null;
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
export function selectablePeriods(historyYears: number): ResolvedPeriodKey[] {
  return [
    ...BASE_PERIOD_KEYS,
    ...LONG_PERIOD_YEARS.filter((period) => historyYears >= period.years).map(
      (period) => period.key,
    ),
    "all",
  ];
}

// Windows "auto" may resolve to. Long windows unlock a little earlier than they
// become selectable (80% of their span) so auto can settle on a bounded window
// instead of jumping straight to "all" for a nearly-that-old book.
export function autoCandidatePeriods(historyYears: number): ResolvedPeriodKey[] {
  return [
    "ytd",
    "1year",
    ...LONG_PERIOD_YEARS.filter(
      (period) => historyYears >= period.years * 0.8,
    ).map((period) => period.key),
    "all",
  ];
}
