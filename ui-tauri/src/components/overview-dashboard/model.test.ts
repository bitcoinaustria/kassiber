import { describe, expect, it, vi } from "vitest";

import { MOCK_OVERVIEW, type OverviewSnapshot } from "@/mocks/seed";

import {
  activityMarkerView,
  activeMarketFiatCurrency,
  activeMarketFiatRate,
  autoFitDomain,
  brushedActivityMarkers,
  buildBalanceRailItems,
  buildHoldingsBySource,
  enrichTreasuryChartData,
  formatBtcAxisFitted,
  formatCompactDisplayMoney,
  formatMarketRateSource,
  formatMarketRateValue,
  formatRelativeMarketRateTime,
  getDataForPeriod,
  isPointInPeriod,
  lastTreasuryLineValue,
  linearAxisTicks,
  logAxisTicks,
  logSafeTreasuryPoints,
  marketRateCompactLabel,
  marketRateDetailLabel,
  marketRateSyncLabel,
  normalizeTimePeriodParam,
  overviewTransactions,
  positiveLogDomain,
  resolveAutoTimePeriod,
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

describe("overview transaction rows", () => {
  it("does not substitute demo rows for an empty live snapshot", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      txs: [],
    };

    expect(overviewTransactions(snapshot)).toEqual([]);
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
      txs: [],
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

describe("chart scale helpers", () => {
  const autoPeriodTx = (id: string, occurredAt: string) => ({
    id,
    date: occurredAt.slice(0, 10),
    occurredAt,
    type: "Income" as const,
    account: "Treasury",
    counter: "External",
    amountSat: 100_000,
    eur: 50,
    rate: 50_000,
    tag: "income",
    conf: 1,
  });

  it("recognizes auto period params and resolves to the tightest active window", () => {
    expect(normalizeTimePeriodParam("auto")).toBe("auto");
    expect(normalizeTimePeriodParam("automatic")).toBe("auto");

    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      portfolioSeries: [
        {
          date: "2026-07-05",
          label: "Jul 5",
          balanceBtc: 1,
          valueEur: 50_000,
          costBasisEur: 45_000,
        },
      ],
      txs: [],
      activityTxs: [
        autoPeriodTx("recent-1", "2026-06-28T12:00:00Z"),
        autoPeriodTx("recent-2", "2026-06-20T12:00:00Z"),
        autoPeriodTx("recent-3", "2026-06-10T12:00:00Z"),
      ],
    };

    expect(resolveAutoTimePeriod(snapshot, "auto")).toBe("30days");
  });

  it("zooms out when recent periods do not contain enough activity", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      portfolioSeries: [
        {
          date: "2026-07-05",
          label: "Jul 5",
          balanceBtc: 1,
          valueEur: 50_000,
          costBasisEur: 45_000,
        },
      ],
      txs: [],
      activityTxs: [
        autoPeriodTx("old-1", "2026-01-20T12:00:00Z"),
        autoPeriodTx("old-2", "2026-01-10T12:00:00Z"),
        autoPeriodTx("old-3", "2025-12-15T12:00:00Z"),
      ],
    };

    expect(resolveAutoTimePeriod(snapshot, "auto")).toBe("1year");
  });

  it("recognizes 6-month period params and windows", () => {
    expect(normalizeTimePeriodParam("6m")).toBe("6months");
    expect(normalizeTimePeriodParam("6months")).toBe("6months");
    expect(normalizeTimePeriodParam("6MO")).toBe("6months");
    const latest = new Date("2026-07-01T00:00:00Z");
    expect(isPointInPeriod("2026-02-01", latest, "6months")).toBe(true);
    expect(isPointInPeriod("2025-12-01", latest, "6months")).toBe(false);
  });

  it("builds a positive multiplicative domain for log axes", () => {
    expect(positiveLogDomain([0, null, undefined, 40, 50])).toEqual([
      40 * 0.96,
      50 * 1.04,
    ]);
    expect(positiveLogDomain([0, -5, null])).toBeNull();
    expect(positiveLogDomain([7])).toEqual([7 * 0.9, 7 * 1.1]);
  });

  it("nulls non-positive values so log scales never see zero", () => {
    const [point] = logSafeTreasuryPoints([
      {
        lineBalanceBtc: 0,
        lineBitcoinPriceEur: 60_000,
        lineAvgCostEur: -1,
        brushBalanceBtc: 0,
      } as never,
    ]);
    expect(point.lineBalanceBtc).toBeUndefined();
    expect(point.lineBitcoinPriceEur).toBe(60_000);
    expect(point.lineAvgCostEur).toBeNull();
    expect(point.brushBalanceBtc).toBe(0);
  });

  it("spaces log ticks evenly in log space with adaptive precision", () => {
    const wide = logAxisTicks([1, 100], 3);
    expect(wide).toEqual([1, 10, 100]);
    const narrow = logAxisTicks([40, 41], 3);
    expect(narrow.length).toBeGreaterThan(1);
    expect(new Set(narrow).size).toBe(narrow.length);
  });

  it("keeps edge ticks inside the domain instead of rounding them out", () => {
    const logTicks = logAxisTicks([39.2, 42.484], 5);
    expect(logTicks[0]).toBeCloseTo(39.2);
    expect(logTicks.at(-1)).toBeLessThanOrEqual(42.484);
    expect(logTicks.at(-1)).toBeGreaterThan(42);
    const linear = linearAxisTicks([44_500, 87_400], 5);
    expect(linear[0]).toBe(45_000);
    expect(linear.at(-1)).toBe(85_000);
    expect(linearAxisTicks([5, 5], 5)).toEqual([]);
  });

  it("fits a padded auto domain and never dips below zero", () => {
    const domain = autoFitDomain([40.2, 40.8, null, undefined]);
    expect(domain).not.toBeNull();
    const [lo, hi] = domain as [number, number];
    expect(lo).toBeLessThan(40.2);
    expect(hi).toBeGreaterThan(40.8);
    expect(autoFitDomain([0.01])?.[0]).toBeGreaterThanOrEqual(0);
    expect(autoFitDomain([null, undefined])).toBeNull();
  });

  it("formats fitted axis ticks with enough precision to distinguish them", () => {
    expect(formatBtcAxisFitted(40.83, [40.8, 40.9])).toBe("₿40.83");
    expect(formatBtcAxisFitted(40.827, [40.82, 40.85])).toBe("₿40.827");
    expect(formatBtcAxisFitted(40.8, [39, 43])).toBe("₿40.8");
    expect(formatBtcAxisFitted(40.8, null)).toBe("₿41");
  });

  it("finds the latest drawable line value for the axis tag", () => {
    const points = [
      { lineBalanceBtc: 1 },
      { lineBalanceBtc: 2 },
      { lineBalanceBtc: undefined },
    ] as never[];
    expect(lastTreasuryLineValue(points, "lineBalanceBtc")).toBe(2);
    expect(lastTreasuryLineValue([], "lineBalanceBtc")).toBeNull();
  });
});
