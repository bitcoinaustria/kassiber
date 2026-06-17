/**
 * Localization config — the single source of truth for which languages the
 * Kassiber desktop UI ships. Adding a language is a one-place change here plus
 * a matching set of resource bundles under `locales/<code>/`.
 *
 * `locale` is the BCP-47 tag used for language-driven `Intl` formatting (dates,
 * language-sensitive number display). It is deliberately separate from the
 * fiat-driven money formatting in `@/lib/currency`, where the locale follows
 * the *currency* (e.g. EUR is always grouped de-AT style) rather than the UI
 * language. See docs/reference/i18n.md for the why.
 *
 * This module is intentionally dependency-free (no i18next, no React) so the
 * UI store and tests can import the language set without pulling in the runtime.
 */

export interface LanguageDefinition {
  /** Stable code persisted in the UI store and used as the i18next language. */
  code: string;
  /** Native, self-describing name shown in the language switcher. */
  label: string;
  /** BCP-47 locale for language-driven `Intl` date/number formatting. */
  locale: string;
}

export const SUPPORTED_LANGUAGES = [
  { code: "en", label: "English", locale: "en-US" },
  { code: "de", label: "Deutsch", locale: "de-AT" },
] as const satisfies readonly LanguageDefinition[];

export type LanguageCode = (typeof SUPPORTED_LANGUAGES)[number]["code"];

export const DEFAULT_LANGUAGE: LanguageCode = "en";

export const SUPPORTED_LANGUAGE_CODES: readonly LanguageCode[] =
  SUPPORTED_LANGUAGES.map((language) => language.code);

/** Narrow an arbitrary value (e.g. a persisted store field) to a known code. */
export function isSupportedLanguage(value: unknown): value is LanguageCode {
  return (
    typeof value === "string" &&
    SUPPORTED_LANGUAGE_CODES.includes(value as LanguageCode)
  );
}

const DEFAULT_LOCALE = SUPPORTED_LANGUAGES.find(
  (language) => language.code === DEFAULT_LANGUAGE,
)!.locale;

/** BCP-47 locale for the active UI language (date/number `Intl` formatting). */
export function localeForLanguage(lang: string): string {
  return (
    SUPPORTED_LANGUAGES.find((language) => language.code === lang)?.locale ??
    DEFAULT_LOCALE
  );
}
