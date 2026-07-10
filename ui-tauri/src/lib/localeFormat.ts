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
