// Keep these bounds in sync with `kassiber.core.ui_snapshot` report-year
// normalization so invalid URL years fail before they reach the daemon.
export const MIN_REPORT_YEAR = 2009;
export const MAX_REPORT_YEAR = 2100;

export function normalizeReportYear(value: string | null): number | null {
  if (!value) return null;
  const parsed = Number(value);
  if (!Number.isInteger(parsed)) return null;
  if (parsed < MIN_REPORT_YEAR || parsed > MAX_REPORT_YEAR) return null;
  return parsed;
}

export function reportYearFromSearch(search: string): number | null {
  return normalizeReportYear(new URLSearchParams(search).get("year"));
}
