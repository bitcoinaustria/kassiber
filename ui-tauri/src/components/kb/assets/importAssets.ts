/**
 * What an exchange sync or provider import left out by asset, and the wallet
 * setting that decides it (docs/plan/24-crypto-assets.md). A wallet's
 * `asset_scope` is `bitcoin` (the default) or `crypto`; importers report the
 * crypto tickers the `bitcoin` scope skipped and the non-crypto holdings
 * (stocks, ETFs, metals) no scope imports.
 */
export type AssetScope = "bitcoin" | "crypto";

export interface ImportAssets {
  scope: AssetScope;
  /** Crypto rows the `bitcoin` scope skipped, by ticker. */
  skipped?: Record<string, number>;
  /** Non-crypto rows no scope imports, by ticker. */
  excluded?: Record<string, number>;
  /**
   * Crypto rows of an exchange Kassiber reads only Bitcoin from (Bitfinex,
   * Bitcoin.de), by ticker: no scope imports them, so no scope is offered.
   */
  unread?: Record<string, number>;
  /** The CLI command that turns the scope on (English, not shown). */
  next_step?: string;
}

export function isAssetScope(value: unknown): value is AssetScope {
  return value === "bitcoin" || value === "crypto";
}

function counts(value: unknown): Record<string, number> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const out: Record<string, number> = {};
  for (const [ticker, rows] of Object.entries(value)) {
    if (typeof rows === "number" && Number.isFinite(rows) && rows > 0) out[ticker] = rows;
  }
  return out;
}

/** The `assets` block of an import or sync result, or null when absent. */
export function readImportAssets(value: unknown): ImportAssets | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (!isAssetScope(record.scope)) return null;
  return {
    scope: record.scope,
    skipped: counts(record.skipped),
    excluded: counts(record.excluded),
    unread: counts(record.unread),
  };
}

export interface TickerSummary {
  /** Tickers by row count, most rows first, then by name. */
  tickers: string[];
  rows: number;
}

export function tickerSummary(map: Record<string, number> | undefined): TickerSummary {
  const entries = Object.entries(counts(map)).sort(
    ([a, left], [b, right]) => right - left || a.localeCompare(b),
  );
  return {
    tickers: entries.map(([ticker]) => ticker),
    rows: entries.reduce((total, [, rows]) => total + rows, 0),
  };
}

/** The tickers to name, with the count left unnamed. */
export function shownTickers(tickers: string[], limit = 5) {
  return {
    shown: tickers.slice(0, limit),
    more: Math.max(0, tickers.length - limit),
  };
}
