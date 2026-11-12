import { localeForLanguage } from "@/i18n/config";
import { useUiStore } from "@/store/ui";

/** Locale for the language currently selected in the desktop UI. */
export function currentUiLocale(): string {
  return localeForLanguage(useUiStore.getState().lang);
}

/** Format a non-currency number using the active UI language. */
export function formatUiNumber(
  value: number,
  options?: Intl.NumberFormatOptions,
  language?: string,
): string {
  const locale = language
    ? localeForLanguage(language)
    : currentUiLocale();
  return value.toLocaleString(locale, options);
}

/** Format an integer count using the active UI language. */
export function formatCount(
  value: number,
  language?: string,
): string {
  return formatUiNumber(value, { maximumFractionDigits: 0 }, language);
}

/** Format a satoshi amount while leaving the caller in control of the unit. */
export function formatSats(
  value: number,
  options: {
    language?: string;
    unit?: "sat" | "sats" | "";
  } = {},
): string {
  const { language, unit = "sats" } = options;
  const amount = formatCount(value, language);
  return unit ? `${amount} ${unit}` : amount;
}

const RELATIVE_TIME_UNITS: ReadonlyArray<[Intl.RelativeTimeFormatUnit, number]> = [
  ["day", 86_400_000],
  ["hour", 3_600_000],
  ["minute", 60_000],
];

/**
 * "2 min. ago" / "vor 2 Min." for an ISO timestamp, in the UI language. Under
 * a minute reads as "now"; a future time (clock skew) reads as now too, never
 * "in 3 min.". Returns null for a missing or unparseable timestamp.
 */
export function formatRelativeTime(
  value: string | null | undefined,
  now: number = Date.now(),
  language?: string,
): string | null {
  if (!value) return null;
  const at = Date.parse(value);
  if (Number.isNaN(at)) return null;
  const elapsed = Math.max(0, now - at);
  const format = new Intl.RelativeTimeFormat(
    language ? localeForLanguage(language) : currentUiLocale(),
    { numeric: "auto", style: "short" },
  );
  for (const [unit, size] of RELATIVE_TIME_UNITS) {
    if (elapsed >= size) return format.format(-Math.floor(elapsed / size), unit);
  }
  return format.format(0, "second");
}

/** The full date and time of an ISO timestamp, in the UI language. */
export function formatDateTime(
  value: string | null | undefined,
  language?: string,
): string | null {
  if (!value) return null;
  const at = Date.parse(value);
  if (Number.isNaN(at)) return null;
  return new Intl.DateTimeFormat(
    language ? localeForLanguage(language) : currentUiLocale(),
    { dateStyle: "medium", timeStyle: "short" },
  ).format(at);
}
