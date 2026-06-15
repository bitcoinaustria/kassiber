/**
 * Pure helpers for the ManySats calculator (Extras → ManySats).
 *
 * The calculator converts between fiat, BTC, and satoshis at the *current*
 * live spot price from the connected market-rate provider. Only the supported
 * live pairs (BTC-USD / BTC-EUR via `ui.rates.latest`) are offered — there is
 * no cached/manual fallback. Keeping the math here (no React) makes it
 * unit-testable and keeps the component focused on layout.
 */

import { localeForFiat } from "@/lib/currency";
import type { RateLatestData } from "@/components/kb/settings/SettingsModel";

export const SATS_PER_BTC = 100_000_000;

export type ConversionField = "fiat" | "btc" | "sats";

/** Fiats with reliable live spot via `ui.rates.latest` (BTC-USD / BTC-EUR). */
export const LIVE_FIATS = ["EUR", "USD"] as const;

export function isLiveFiat(code: string): boolean {
  const upper = code.trim().toUpperCase();
  return (LIVE_FIATS as readonly string[]).includes(upper);
}

/** Build the `BTC-<FIAT>` pair key for a fiat code. */
export function pairForFiat(code: string): string {
  return `BTC-${code.trim().toUpperCase()}`;
}

/**
 * Parse a loosely-typed numeric string into a number, tolerating both en
 * (`1,234.56`) and de (`1.234,56`) grouping. The separator that appears last
 * is treated as the decimal point; a lone comma is a decimal separator.
 * Returns null for empty or non-numeric input.
 */
export function parseLooseNumber(raw: string): number | null {
  let s = raw.trim().replace(/[\s_]/g, "");
  if (!s) return null;
  const hasComma = s.includes(",");
  const hasDot = s.includes(".");
  if (hasComma && hasDot) {
    if (s.lastIndexOf(",") > s.lastIndexOf(".")) {
      s = s.replace(/\./g, "").replace(/,/g, ".");
    } else {
      s = s.replace(/,/g, "");
    }
  } else if (hasComma) {
    const commaCount = (s.match(/,/g) ?? []).length;
    s = commaCount > 1 ? s.replace(/,/g, "") : s.replace(",", ".");
  } else if (hasDot) {
    const dotCount = (s.match(/\./g) ?? []).length;
    if (dotCount > 1) s = s.replace(/\./g, "");
  }
  if (!/^[-+]?(\d+\.?\d*|\.\d+)$/.test(s)) return null;
  const value = Number(s);
  return Number.isFinite(value) ? value : null;
}

/**
 * Resolve the canonical BTC amount from the value the user typed into one
 * field. Fiat → BTC needs a price; without one it returns null.
 */
export function deriveBtc(
  field: ConversionField,
  value: number,
  price: number | null,
): number | null {
  if (field === "btc") return value;
  if (field === "sats") return value / SATS_PER_BTC;
  if (price == null || price <= 0) return null;
  return value / price;
}

/** Plain BTC string for inputs/copy: up to 8 decimals, trailing zeros trimmed. */
export function formatBtcPlain(btc: number): string {
  if (!Number.isFinite(btc)) return "";
  if (btc === 0) return "0";
  let s = btc.toFixed(8);
  if (s.includes(".")) s = s.replace(/0+$/, "").replace(/\.$/, "");
  return s;
}

/** Plain integer-sats string for inputs/copy. */
export function formatSatsPlain(btc: number): string {
  if (!Number.isFinite(btc)) return "";
  return String(Math.round(btc * SATS_PER_BTC));
}

const fiatFractionDigitsCache = new Map<string, number>();

/** Currency-correct decimal count (JPY → 0, most → 2), derived from Intl. */
export function fiatFractionDigits(code: string): number {
  const key = code.trim().toUpperCase();
  const cached = fiatFractionDigitsCache.get(key);
  if (cached !== undefined) return cached;
  let digits: number;
  try {
    digits =
      new Intl.NumberFormat(localeForFiat(key), {
        style: "currency",
        currency: key,
      }).resolvedOptions().maximumFractionDigits ?? 2;
  } catch {
    digits = 2;
  }
  fiatFractionDigitsCache.set(key, digits);
  return digits;
}

/** Plain fiat string for inputs/copy (no symbol, currency-correct decimals). */
export function formatFiatPlain(value: number, code: string): string {
  if (!Number.isFinite(value)) return "";
  return value.toFixed(fiatFractionDigits(code));
}

export interface ResolvedRate {
  /** Fiat per 1 BTC, or null when no rate is available. */
  price: number | null;
  fiatCurrency: string | null;
  source: string | null;
  timestamp: string | null;
  fetchedAt: string | null;
}

/** Resolve the live rate from a `ui.rates.latest` payload. */
export function rateFromLatest(data: RateLatestData | undefined): ResolvedRate | null {
  const marketRate = data?.marketRate;
  if (!marketRate) return null;
  return {
    price: marketRate.rate ?? null,
    fiatCurrency: marketRate.fiatCurrency ?? null,
    source: marketRate.source ?? null,
    timestamp: marketRate.timestamp ?? null,
    fetchedAt: marketRate.fetchedAt ?? null,
  };
}
