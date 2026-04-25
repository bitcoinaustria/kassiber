/**
 * Currency helpers — render BTC, sats, or EUR based on the AppHeader's
 * ₿/€ toggle. The canonical formatting is hoisted from
 * routes/Overview.tsx (fmtBtc / fmtEur / formatSat) so all screens read
 * the same way.
 *
 * Inputs are always BTC scalars; helpers convert to the target unit so
 * callers never have to keep a parallel sats/EUR copy in scope.
 */

import { useUiStore } from "@/store/ui";

export type Currency = "btc" | "eur";

export const useCurrency = () => useUiStore((s) => s.currency);

interface FormatBtcOpts {
  /** Decimal precision; defaults to 8 (full sat resolution). */
  precision?: number;
  /** Render an explicit `+ ` / `− ` sign (uses unicode minus). */
  sign?: boolean;
}

interface FormatEurOpts {
  /** Decimal precision; defaults to 2. */
  precision?: number;
  /** Render an explicit `+ ` / `− ` sign (uses unicode minus). */
  sign?: boolean;
}

interface FormatSatsOpts {
  /** Render an explicit `+ ` / `− ` sign (uses unicode minus). */
  sign?: boolean;
}

const signedPrefix = (n: number) => (n >= 0 ? "+ " : "− ");

/** Format a BTC scalar as `₿ X.XXXXXXXX` (or to a custom precision). */
export function formatBtc(btc: number, opts: FormatBtcOpts = {}): string {
  const { precision = 8, sign = false } = opts;
  const abs = Math.abs(btc).toFixed(precision);
  const prefix = sign ? signedPrefix(btc) : "";
  return prefix + "₿ " + abs;
}

/**
 * Format a BTC scalar as the equivalent EUR amount given a spot price.
 * Uses de-AT locale grouping (`€ 12.345,67`) by default.
 */
export function formatEur(
  btc: number,
  priceEur: number,
  opts: FormatEurOpts = {},
): string {
  const { precision = 2, sign = false } = opts;
  const eur = btc * priceEur;
  const abs = Math.abs(eur).toLocaleString("de-AT", {
    minimumFractionDigits: precision,
    maximumFractionDigits: precision,
  });
  const prefix = sign ? signedPrefix(eur) : "";
  return prefix + "€ " + abs;
}

/** Format a BTC scalar as integer sats with locale grouping. */
export function formatSats(btc: number, opts: FormatSatsOpts = {}): string {
  const { sign = false } = opts;
  const sats = Math.round(btc * 1e8);
  const abs = Math.abs(sats).toLocaleString("en-US");
  const prefix = sign ? signedPrefix(sats) : "";
  return prefix + abs;
}

interface FmtCcyOpts {
  sign?: boolean;
}

/**
 * Currency-aware single-line formatter — chooses BTC or EUR based on
 * the user's toggle. Use for places where the same slot has to swap
 * between the two units.
 */
export function fmtCcy(
  btc: number,
  currency: Currency,
  priceEur: number,
  opts: FmtCcyOpts = {},
): string {
  if (currency === "eur") return formatEur(btc, priceEur, opts);
  return formatBtc(btc, opts);
}
