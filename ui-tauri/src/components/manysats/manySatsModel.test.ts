import { describe, expect, it } from "vitest";

import {
  applyPremium,
  clampPremium,
  deriveBtc,
  formatBtcPlain,
  formatFiatPlain,
  formatSatsPlain,
  isLiveFiat,
  LIVE_FIATS,
  pairForFiat,
  parseFieldAmount,
  rateFromLatest,
  SATS_PER_BTC,
} from "./manySatsModel";

describe("fiat pair helpers", () => {
  it("maps fiat codes to BTC pairs", () => {
    expect(pairForFiat("eur")).toBe("BTC-EUR");
    expect(pairForFiat(" usd ")).toBe("BTC-USD");
  });

  it("only treats the live-supported pairs as live", () => {
    expect(LIVE_FIATS).toEqual(["EUR", "USD"]);
    expect(isLiveFiat("usd")).toBe(true);
    expect(isLiveFiat("EUR")).toBe(true);
    expect(isLiveFiat("GBP")).toBe(false);
  });
});

describe("parseFieldAmount", () => {
  it("returns null for empty or junk input", () => {
    expect(parseFieldAmount("", "fiat")).toBeNull();
    expect(parseFieldAmount("   ", "btc")).toBeNull();
    expect(parseFieldAmount("abc", "sats")).toBeNull();
    expect(parseFieldAmount("1.2.3,4,5x", "fiat")).toBeNull();
  });

  it("reads a single thousands separator in fiat as grouping (en + de)", () => {
    // A lone separator + three digits is a thousands group either way.
    expect(parseFieldAmount("1.000", "fiat")).toBe(1000);
    expect(parseFieldAmount("1,000", "fiat")).toBe(1000);
    expect(parseFieldAmount("12.000", "fiat")).toBe(12000);
    // 1–2 trailing digits stay decimal, so the 2-decimal display round-trips.
    expect(parseFieldAmount("71545.43", "fiat")).toBe(71545.43);
    expect(parseFieldAmount("1,50", "fiat")).toBe(1.5);
    expect(parseFieldAmount("1.50", "fiat")).toBe(1.5);
    // Mixed separators: the last one is the decimal point.
    expect(parseFieldAmount("1,234.56", "fiat")).toBe(1234.56);
    expect(parseFieldAmount("1.234,56", "fiat")).toBe(1234.56);
    // Repeated grouping separators.
    expect(parseFieldAmount("1.234.567", "fiat")).toBe(1234567);
    expect(parseFieldAmount("1,234,567", "fiat")).toBe(1234567);
    // Leading-zero integer is a decimal, not a group.
    expect(parseFieldAmount("0.500", "fiat")).toBe(0.5);
  });

  it("treats every separator in sats as grouping (integers)", () => {
    expect(parseFieldAmount("1,000", "sats")).toBe(1000);
    expect(parseFieldAmount("100.000.000", "sats")).toBe(100000000);
    expect(parseFieldAmount("100,000,000", "sats")).toBe(100000000);
    expect(parseFieldAmount("153846", "sats")).toBe(153846);
  });

  it("treats '.' as the decimal point for BTC and keeps de comma decimals", () => {
    expect(parseFieldAmount("0.5", "btc")).toBe(0.5);
    expect(parseFieldAmount("0.500", "btc")).toBe(0.5);
    expect(parseFieldAmount("1.500", "btc")).toBe(1.5);
    expect(parseFieldAmount("0,5", "btc")).toBe(0.5);
    expect(parseFieldAmount("1.000", "btc")).toBe(1);
    expect(parseFieldAmount("0.00153846", "btc")).toBe(0.00153846);
    expect(parseFieldAmount("1,000.5", "btc")).toBe(1000.5);
  });

  it("ignores spaces and underscores as group separators", () => {
    expect(parseFieldAmount("1 234 567", "fiat")).toBe(1234567);
    expect(parseFieldAmount("100_000", "sats")).toBe(100000);
  });
});

describe("deriveBtc", () => {
  it("derives BTC from sats and BTC directly without a price", () => {
    expect(deriveBtc("btc", 1.5, null)).toBe(1.5);
    expect(deriveBtc("sats", SATS_PER_BTC, null)).toBe(1);
    expect(deriveBtc("sats", 153_846, 65_000)).toBeCloseTo(0.00153846, 12);
  });

  it("derives BTC from fiat only when a positive price exists", () => {
    expect(deriveBtc("fiat", 65_000, 65_000)).toBe(1);
    expect(deriveBtc("fiat", 100, null)).toBeNull();
    expect(deriveBtc("fiat", 100, 0)).toBeNull();
  });
});

describe("premium / discount", () => {
  it("clamps to the allowed range and treats non-finite as 0", () => {
    expect(clampPremium(2.5)).toBe(2.5);
    expect(clampPremium(-3)).toBe(-3);
    expect(clampPremium(999)).toBe(10);
    expect(clampPremium(-999)).toBe(-10);
    expect(clampPremium(Number.NaN)).toBe(0);
  });

  it("applies a premium or discount over the market price", () => {
    expect(applyPremium(100, 0)).toBe(100);
    expect(applyPremium(100, 2)).toBeCloseTo(102, 9);
    expect(applyPremium(100, -1.5)).toBeCloseTo(98.5, 9);
    expect(applyPremium(null, 2)).toBeNull();
    expect(applyPremium(0, 2)).toBeNull();
  });
});

describe("plain formatters", () => {
  it("formats BTC trimming trailing zeros", () => {
    expect(formatBtcPlain(0)).toBe("0");
    expect(formatBtcPlain(1.5)).toBe("1.5");
    expect(formatBtcPlain(0.00153846)).toBe("0.00153846");
    expect(formatBtcPlain(0.00000001)).toBe("0.00000001");
  });

  it("formats sats as a rounded integer", () => {
    expect(formatSatsPlain(1)).toBe("100000000");
    expect(formatSatsPlain(0.00153846)).toBe("153846");
  });

  it("formats fiat with currency-correct decimals", () => {
    expect(formatFiatPlain(1234.5, "EUR")).toBe("1234.50");
    expect(formatFiatPlain(1234.5, "JPY")).toBe("1235");
  });
});

describe("rate resolution", () => {
  it("reads the live rate from a ui.rates.latest payload", () => {
    const resolved = rateFromLatest({
      source: "coinbase-exchange",
      pair: "BTC-EUR",
      latest: [],
      marketRate: {
        asset: "BTC",
        fiatCurrency: "EUR",
        pair: "BTC-EUR",
        rate: 71_420.18,
        timestamp: "2026-06-15T10:00:00Z",
        source: "coinbase-exchange",
        fetchedAt: "2026-06-15T10:01:00Z",
        granularity: "minute",
        method: "product_candles",
      },
    });
    expect(resolved?.price).toBe(71_420.18);
    expect(resolved?.fiatCurrency).toBe("EUR");
    expect(resolved?.source).toBe("coinbase-exchange");
  });

  it("returns null when there is no market rate", () => {
    expect(
      rateFromLatest({
        source: "coinbase-exchange",
        pair: "BTC-EUR",
        latest: [],
        marketRate: null,
      }),
    ).toBeNull();
    expect(rateFromLatest(undefined)).toBeNull();
  });
});
