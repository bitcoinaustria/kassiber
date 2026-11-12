// The tax countries the backend supports, read from the Rust registry's
// generated copy (crates/tax-countries). Pickers, defaults and the preview
// badge come from here so the desktop never lists a country the backend
// refuses, or misses one it accepts.
import { TAX_COUNTRY_REGISTRY } from "./taxCountries.generated";

export type TaxCountryEntry = (typeof TAX_COUNTRY_REGISTRY)[number];
export type TaxCountry = TaxCountryEntry["code"];

// Every gains algorithm token any country allows (`kassiber.tax_policy`).
export type GainsAlgorithm = TaxCountryEntry["methods"][number];

export const TAX_COUNTRIES: TaxCountry[] = TAX_COUNTRY_REGISTRY.map(
  (entry) => entry.code,
);

export const taxCountryEntry = (
  code: string | null | undefined,
): TaxCountryEntry | undefined => {
  const normalized = String(code ?? "").trim().toLowerCase();
  return TAX_COUNTRY_REGISTRY.find((entry) => entry.code === normalized);
};

export const isTaxCountry = (code: string | null | undefined): code is TaxCountry =>
  taxCountryEntry(code) !== undefined;

// Not yet reviewed by a tax professional for the country.
export const isPreviewTaxCountry = (code: string | null | undefined): boolean =>
  Boolean(taxCountryEntry(code)?.preview);

// The country's name in the UI language ("de" for German, else English).
export const taxCountryName = (
  code: string | null | undefined,
  language: string,
): string => {
  const entry = taxCountryEntry(code);
  if (!entry) return String(code ?? "");
  return language.toLowerCase().startsWith("de") ? entry.name.de : entry.name.en;
};

// The order pickers list methods in: the country's default first, then the
// lot methods. A country that has Austria's moving average does not offer
// the plain one in the picker; the backend still accepts it.
const PICKER_ORDER: GainsAlgorithm[] = [
  "FIFO",
  "LIFO",
  "HIFO",
  "LOFO",
  "MOVING_AVERAGE",
  "MOVING_AVERAGE_AT",
  "MOVING_AVERAGE_FIFO_PERIOD",
  "UK_SHARE_MATCHING",
  "JP_TOTAL_AVERAGE",
  "JP_MOVING_AVERAGE",
];

export const gainsAlgorithmsFor = (country: TaxCountry): GainsAlgorithm[] => {
  const entry = taxCountryEntry(country);
  if (!entry) return [];
  const allowed = new Set<GainsAlgorithm>(entry.methods);
  const fallback = entry.default_method as GainsAlgorithm;
  return [
    fallback,
    ...PICKER_ORDER.filter(
      (method) =>
        method !== fallback &&
        allowed.has(method) &&
        !(method === "MOVING_AVERAGE" && allowed.has("MOVING_AVERAGE_AT")),
    ),
  ];
};

export const defaultGainsAlgorithm = (country: TaxCountry): GainsAlgorithm =>
  (taxCountryEntry(country)?.default_method ?? "FIFO") as GainsAlgorithm;

// The fiat currency the country requires, or null when the book chooses.
export const requiredFiatCurrency = (
  country: string | null | undefined,
): string | null => taxCountryEntry(country)?.fiat_currency ?? null;

// Whether the book sets its own long-term holding period in days.
export const usesLongTermDays = (country: string | null | undefined): boolean =>
  taxCountryEntry(country)?.long_term.kind === "profile_days";

export const defaultPoolScope = (country: string | null | undefined): string =>
  taxCountryEntry(country)?.default_pool_scope ?? "global";
