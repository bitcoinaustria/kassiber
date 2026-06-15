import { describe, expect, it } from "vitest";

import {
  deriveBtc,
  formatBtcPlain,
  formatFiatPlain,
  formatSatsPlain,
  isLiveFiat,
  LIVE_FIATS,
  pairForFiat,
  parseLooseNumber,
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

describe("parseLooseNumber", () => {
  it("returns null for empty or junk input", () => {
    expect(parseLooseNumber("")).toBeNull();
    expect(parseLooseNumber("   ")).toBeNull();
    expect(parseLooseNumber("abc")).toBeNull();
    expect(parseLooseNumber("1.2.3,4,5x")).toBeNull();
  });

  it("parses plain and en-grouped numbers", () => {
    expect(parseLooseNumber("1234.56")).toBe(1234.56);
    expect(parseLooseNumber("1,234.56")).toBe(1234.56);
    expect(parseLooseNumber("1,234,567")).toBe(1234567);
    expect(parseLooseNumber("100")).toBe(100);
  });

  it("parses de-grouped numbers (last separator wins)", () => {
    expect(parseLooseNumber("1.234,56")).toBe(1234.56);
    expect(parseLooseNumber("1234,56")).toBe(1234.56);
    expect(parseLooseNumber("1.234.567")).toBe(1234567);
    expect(parseLooseNumber("0,5")).toBe(0.5);
  });

  it("ignores spaces and underscores as group separators", () => {
    expect(parseLooseNumber("1 234 567")).toBe(1234567);
    expect(parseLooseNumber("100_000")).toBe(100000);
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
