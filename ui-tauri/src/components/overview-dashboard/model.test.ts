import { describe, expect, it, vi } from "vitest";

import { MOCK_OVERVIEW, type OverviewSnapshot } from "@/mocks/seed";

import {
  activeMarketFiatCurrency,
  formatCompactDisplayMoney,
  formatMarketRateSource,
  formatMarketRateValue,
  formatRelativeMarketRateTime,
  marketRateCompactLabel,
  marketRateDetailLabel,
  marketRateSyncLabel,
} from "./model";

describe("overview market rate display", () => {
  it("formats the active book fiat rate with source and sync metadata", () => {
    const now = new Date("2026-03-01T00:04:30Z");
    vi.useFakeTimers();
    vi.setSystemTime(now);

    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      fiat: { ...MOCK_OVERVIEW.fiat, fiatCurrency: "USD" },
      marketRate: {
        asset: "BTC",
        fiatCurrency: "USD",
        pair: "BTC-USD",
        rate: 70_000,
        timestamp: "2026-03-01T00:00:00Z",
        source: "coingecko",
        fetchedAt: "2026-03-01T00:02:00Z",
        granularity: "daily",
        method: "close",
      },
    };

    expect(activeMarketFiatCurrency(snapshot)).toBe("USD");
    expect(formatMarketRateValue(snapshot)).toBe("$70,000.00 / BTC");
    expect(marketRateCompactLabel(snapshot)).toBe("CoinGecko · 2m ago");
    expect(marketRateSyncLabel(snapshot)).toBe("Synced 2026-03-01 00:02");
    expect(marketRateDetailLabel(snapshot)).toBe("CoinGecko · BTC-USD");
    expect(formatCompactDisplayMoney(140_000, 70_000, "eur", "USD")).toBe(
      "$140K",
    );
    expect(formatCompactDisplayMoney(140_000, 70_000, "btc", "USD")).toBe(
      "₿ 2.000",
    );

    vi.useRealTimers();
  });

  it("falls back to the book fiat when no cached rate is available", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      fiat: { ...MOCK_OVERVIEW.fiat, fiatCurrency: "CHF" },
      marketRate: {
        asset: "BTC",
        fiatCurrency: "CHF",
        pair: null,
        rate: null,
        timestamp: null,
        source: null,
        fetchedAt: null,
        granularity: null,
        method: null,
      },
    };

    expect(activeMarketFiatCurrency(snapshot)).toBe("CHF");
    expect(formatMarketRateValue(snapshot)).toBe("No CHF rate");
    expect(marketRateCompactLabel(snapshot)).toBe("Fetch rates");
    expect(marketRateSyncLabel(snapshot)).toBe("Not synced");
    expect(marketRateDetailLabel(snapshot)).toBe("Fetch rates");
  });

  it("uses friendly labels for known rate sources", () => {
    expect(formatMarketRateSource("coinbase-exchange")).toBe("Coinbase Exchange");
    expect(formatMarketRateSource("kraken-csv")).toBe("Kraken CSV");
    expect(formatMarketRateSource("manual")).toBe("Manual");
  });

  it("formats compact relative sync times", () => {
    const now = Date.parse("2026-03-01T12:00:00Z");

    expect(formatRelativeMarketRateTime("2026-03-01T11:59:40Z", now)).toBe(
      "just now",
    );
    expect(formatRelativeMarketRateTime("2026-03-01T11:57:00Z", now)).toBe(
      "3m ago",
    );
    expect(formatRelativeMarketRateTime("2026-03-01T09:15:00Z", now)).toBe(
      "2h ago",
    );
    expect(formatRelativeMarketRateTime("2026-02-27T12:00:00Z", now)).toBe(
      "2d ago",
    );
  });
});
