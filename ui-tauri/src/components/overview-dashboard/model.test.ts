import { describe, expect, it, vi } from "vitest";

import { MOCK_OVERVIEW, type OverviewSnapshot } from "@/mocks/seed";

import {
  activityMarkerView,
  activeMarketFiatCurrency,
  activeMarketFiatRate,
  brushedActivityMarkers,
  buildBalanceRailItems,
  buildHoldingsBySource,
  enrichTreasuryChartData,
  formatCompactDisplayMoney,
  formatMarketRateSource,
  formatMarketRateValue,
  formatRelativeMarketRateTime,
  getDataForPeriod,
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
    expect(activeMarketFiatRate(snapshot)).toBe(70_000);
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

  it("uses the active book fiat rate for overview BTC balance conversions", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      priceEur: 65_000,
      priceUsd: 70_000,
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
      connections: [
        {
          ...MOCK_OVERVIEW.connections[0],
          kind: "xpub",
          label: "Cold Storage",
          balance: 2,
        },
        {
          ...MOCK_OVERVIEW.connections[2],
          kind: "core-ln",
          label: "Home Node",
          balance: 0.5,
        },
      ],
    };

    expect(activeMarketFiatRate(snapshot)).toBe(70_000);
    expect(buildHoldingsBySource(snapshot).map(({ name, value }) => [name, value]))
      .toEqual([
        ["Cold Storage", 140_000],
        ["Home Node", 35_000],
      ]);
    expect(buildBalanceRailItems(snapshot).total).toBe(175_000);
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

describe("overview treasury chart", () => {
  it("uses explicit daily BTC prices from portfolio points", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      portfolioSeries: [
        {
          date: "2026-01-01",
          label: "2026-01-01",
          balanceBtc: 0,
          valueEur: 0,
          costBasisEur: 0,
          priceEur: 60_000,
        },
      ],
      activityTxs: [],
    };

    const points = enrichTreasuryChartData(
      getDataForPeriod("all", snapshot, "value", "eur", "detailed"),
      snapshot,
      "all",
    );

    expect(points[0]?.lineBitcoinPriceEur).toBe(60_000);
  });

  it("keeps transaction prices and event balances out of long-range overview lines", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      portfolioSeries: [
        {
          date: "2026-01-01",
          label: "2026-01-01",
          balanceBtc: 1,
          valueEur: 100_000,
          costBasisEur: 80_000,
          priceEur: 100_000,
        },
        {
          date: "2026-01-02",
          label: "2026-01-02",
          balanceBtc: 1.1,
          valueEur: 121_000,
          costBasisEur: 85_000,
          priceEur: 110_000,
        },
        {
          date: "2026-01-03",
          label: "2026-01-03",
          balanceBtc: 1.1,
          valueEur: 132_000,
          costBasisEur: 85_000,
          priceEur: 120_000,
        },
      ],
      activityTxs: [
        {
          id: "tx-event",
          date: "2026-01-02 12:00",
          occurredAt: "2026-01-02T12:00:00Z",
          type: "Income",
          account: "Treasury",
          counter: "Event-priced invoice",
          amountSat: 10_000_000,
          eur: 5_000,
          rate: 50_000,
          tag: "Revenue",
          conf: 6,
          balanceBtc: 1.05,
          costBasisEur: 84_000,
        },
      ],
    };

    const points = enrichTreasuryChartData(
      getDataForPeriod("all", snapshot, "value", "eur", "detailed"),
      snapshot,
      "all",
    );
    const eventPoint = points.find((point) => point.isActivityEvent);

    expect(points.map((point) => (point.isActivityEvent ? "event" : point.date))).toEqual([
      "2026-01-01",
      "event",
      "2026-01-02",
      "2026-01-03",
    ]);
    expect(points.map((point) => point.lineBitcoinPriceEur)).toEqual([
      100_000,
      undefined,
      110_000,
      120_000,
    ]);
    expect(points.map((point) => point.lineBalanceBtc)).toEqual([
      1,
      undefined,
      1.1,
      1.1,
    ]);
    expect(eventPoint?.lineBalanceBtc).toBeUndefined();
    expect(eventPoint?.lineBitcoinPriceEur).toBeUndefined();
    expect(eventPoint?.bitcoinPriceEur).toBe(50_000);
    expect(eventPoint?.eventPriceEur).toBe(50_000);
    expect(eventPoint?.eventBalanceBtc).toBe(1.05);
    expect(eventPoint?.markerBalanceBtc).toBe(1.1);
    expect(eventPoint?.lineAvgCostEur).toBeUndefined();

    const markerView = activityMarkerView(points, true, () => 0, false);

    expect(markerView.chartDisplayData.some((point) => point.isActivityEvent)).toBe(
      false,
    );
    expect(markerView.visibleActivityMarkers[0]?.markerBalanceBtc).toBe(1.1);
  });

  it("clips activity markers to the brushed chart dates", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      portfolioSeries: [
        {
          date: "2026-01-01",
          label: "2026-01-01",
          balanceBtc: 1,
          valueEur: 100_000,
          costBasisEur: 80_000,
          priceEur: 100_000,
        },
        {
          date: "2026-01-02",
          label: "2026-01-02",
          balanceBtc: 1.1,
          valueEur: 121_000,
          costBasisEur: 85_000,
          priceEur: 110_000,
        },
        {
          date: "2026-01-03",
          label: "2026-01-03",
          balanceBtc: 1.2,
          valueEur: 144_000,
          costBasisEur: 90_000,
          priceEur: 120_000,
        },
      ],
      activityTxs: [
        {
          id: "tx-jan-02",
          date: "2026-01-02 12:00",
          occurredAt: "2026-01-02T12:00:00Z",
          type: "Income",
          account: "Treasury",
          counter: "Event 1",
          amountSat: 10_000_000,
          eur: 5_000,
          rate: 50_000,
          tag: "Revenue",
          conf: 6,
          balanceBtc: 1.1,
          costBasisEur: 85_000,
        },
        {
          id: "tx-jan-03",
          date: "2026-01-03 12:00",
          occurredAt: "2026-01-03T12:00:00Z",
          type: "Income",
          account: "Treasury",
          counter: "Event 2",
          amountSat: 10_000_000,
          eur: 6_000,
          rate: 60_000,
          tag: "Revenue",
          conf: 6,
          balanceBtc: 1.2,
          costBasisEur: 90_000,
        },
      ],
    };

    const points = enrichTreasuryChartData(
      getDataForPeriod("all", snapshot, "value", "eur", "detailed"),
      snapshot,
      "all",
    );
    const markerView = activityMarkerView(points, true, () => 0, false);
    const selectedDisplayData = markerView.chartDisplayData.filter(
      (point) => point.date === "2026-01-03",
    );

    expect(
      brushedActivityMarkers(
        markerView.visibleActivityMarkers,
        selectedDisplayData,
      ).map((point) => point.eventTransactionId),
    ).toEqual(["tx-jan-03"]);
  });

  it("uses event balances for 30-day detail lines", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      portfolioSeries: [
        {
          date: "2026-01-01",
          label: "2026-01-01",
          balanceBtc: 1,
          valueEur: 100_000,
          costBasisEur: 80_000,
          priceEur: 100_000,
        },
        {
          date: "2026-01-02",
          label: "2026-01-02",
          balanceBtc: 1.1,
          valueEur: 121_000,
          costBasisEur: 85_000,
          priceEur: 110_000,
        },
        {
          date: "2026-01-03",
          label: "2026-01-03",
          balanceBtc: 1.1,
          valueEur: 132_000,
          costBasisEur: 85_000,
          priceEur: 120_000,
        },
      ],
      activityTxs: [
        {
          id: "tx-event",
          date: "2026-01-02 12:00",
          occurredAt: "2026-01-02T12:00:00Z",
          type: "Income",
          account: "Treasury",
          counter: "Event-priced invoice",
          amountSat: 10_000_000,
          eur: 5_000,
          rate: 50_000,
          tag: "Revenue",
          conf: 6,
          balanceBtc: 1.1,
          costBasisEur: 85_000,
        },
      ],
    };

    const points = enrichTreasuryChartData(
      getDataForPeriod("30days", snapshot, "value", "eur", "detailed"),
      snapshot,
      "30days",
    );
    const eventPoint = points.find((point) => point.isActivityEvent);

    expect(points.map((point) => point.lineBalanceBtc)).toEqual([
      1,
      1.1,
      1.1,
      1.1,
    ]);
    expect(eventPoint?.markerBalanceBtc).toBe(1.1);
    expect(eventPoint?.lineAvgCostEur).toBeCloseTo(85_000 / 1.1);

    const markerView = activityMarkerView(points, true, () => 0, true);

    expect(markerView.chartDisplayData.some((point) => point.isActivityEvent)).toBe(
      true,
    );
  });
});
