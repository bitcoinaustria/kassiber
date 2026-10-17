/**
 * Export periods as the desktop names them. The core resolves the exact
 * bounds in the book's calendar (``kassiber/core/export_range.py``).
 */
export const PERIOD_PRESETS = [
  "all",
  "today",
  "yesterday",
  "this_week",
  "last_week",
  "this_month",
  "last_month",
  "this_quarter",
  "last_quarter",
  "this_year",
  "last_year",
] as const;

export type PeriodPreset = (typeof PERIOD_PRESETS)[number] | "custom";

export interface ExportPeriod {
  preset: PeriodPreset;
  /** Inclusive days for a custom range, YYYY-MM-DD. */
  from?: string;
  to?: string;
  /** IANA timezone; unset means the book's calendar. */
  timezone?: string;
}

/** The resolved range the core reports back with a preview. */
export interface ResolvedPeriod {
  timezone: string;
  first_day: string;
  last_day: string;
}

/** Daemon arguments for a period: none for every transaction. */
export function periodArgs(period: ExportPeriod): Record<string, string> {
  const args: Record<string, string> =
    period.preset === "all"
      ? {}
      : period.preset === "custom"
        ? { from: period.from ?? "", to: period.to ?? "" }
        : { range: period.preset };
  if (period.timezone && period.preset !== "all") args.timezone = period.timezone;
  return args;
}
