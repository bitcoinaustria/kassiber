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

const NUMERIC_RE = /^[-+]?(\d+\.?\d*|\.\d+)$/;

function toNumberOrNull(normalized: string): number | null {
  if (!NUMERIC_RE.test(normalized)) return null;
  const value = Number(normalized);
  return Number.isFinite(value) ? value : null;
}

/**
 * Normalize loosely-typed en/de input to a single `.`-decimal string,
 * locale-agnostically:
 *  - mixed `.` and `,` → the last-occurring one is the decimal point, the
 *    other is grouping (`1,234.56` and `1.234,56` both ⇒ `1234.56`);
 *  - the same separator repeated → grouping (`1.234.567` ⇒ `1234567`);
 *  - a single separator → decimal, EXCEPT (when `singleTripletIsGroup`) a
 *    separator followed by exactly three digits behind a non-zero integer,
 *    which is read as a thousands group (`1.000`/`1,000` ⇒ `1000`).
 *
 * `singleTripletIsGroup` is true for fiat (grouping is common; values display
 * with 2 decimals so `71545.43` still parses as a decimal) and false for BTC,
 * where `.` is always the decimal and grouping is unrealistic (`0.500` ⇒ 0.5).
 */
function normalizeNumeric(s: string, singleTripletIsGroup: boolean): string {
  const hasComma = s.includes(",");
  const hasDot = s.includes(".");
  if (hasComma && hasDot) {
    return s.lastIndexOf(",") > s.lastIndexOf(".")
      ? s.replace(/\./g, "").replace(/,/g, ".")
      : s.replace(/,/g, "");
  }
  const sep = hasComma ? "," : hasDot ? "." : "";
  if (!sep) return s;
  const count = sep === "," ? (s.match(/,/g) ?? []).length : (s.match(/\./g) ?? []).length;
  if (count > 1) return s.split(sep).join("");
  const idx = s.indexOf(sep);
  const before = s.slice(0, idx);
  const after = s.slice(idx + 1);
  if (singleTripletIsGroup && after.length === 3 && /^[1-9]\d*$/.test(before)) {
    return before + after;
  }
  return sep === "," ? s.replace(",", ".") : s;
}

/**
 * Parse a user-entered amount for a specific field, resolving the
 * decimal/grouping ambiguity per field so grouped input like `1,000` (USD)
 * or `1.000` (de-AT EUR) reads as one thousand rather than 1.0:
 *
 *  - `sats` — integers; every `.`/`,` is a grouping separator and is stripped.
 *  - `fiat` — a lone separator + three digits is a thousands group; 1–2
 *    trailing digits stay decimal (so the 2-decimal display round-trips).
 *  - `btc`  — `.` is always the decimal separator and grouping is unrealistic
 *    (keeps `0,5` ⇒ 0.5 and `0.500` ⇒ 0.5).
 *
 * Returns null for empty or non-numeric input.
 */
export function parseFieldAmount(
  raw: string,
  field: ConversionField,
): number | null {
  const s = raw.trim().replace(/[\s_]/g, "");
  if (!s) return null;
  if (field === "sats") return toNumberOrNull(s.replace(/[.,]/g, ""));
  return toNumberOrNull(normalizeNumeric(s, field === "fiat"));
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

/** Max premium/discount magnitude (percent) the slider and input allow. */
export const PREMIUM_LIMIT_PCT = 10;

/** Clamp a premium/discount percent into [-PREMIUM_LIMIT_PCT, +PREMIUM_LIMIT_PCT]. */
export function clampPremium(pct: number): number {
  if (!Number.isFinite(pct)) return 0;
  return Math.max(-PREMIUM_LIMIT_PCT, Math.min(PREMIUM_LIMIT_PCT, pct));
}

/**
 * Apply a premium (positive) or discount (negative) percent over the market
 * price. Returns null when there is no usable market price.
 */
export function applyPremium(
  price: number | null,
  premiumPct: number,
): number | null {
  if (price == null || !(price > 0)) return null;
  return price * (1 + premiumPct / 100);
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
