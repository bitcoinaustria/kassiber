import { createElement } from "react";
import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

vi.mock("@tanstack/react-router", async () => {
  const React = await import("react");
  return {
    Link: ({
      children,
      className,
      to,
    }: {
      children?: ReactNode;
      className?: string;
      to?: string;
    }) =>
      React.createElement(
        "a",
        { className, href: typeof to === "string" ? to : "#" },
        children,
      ),
  };
});

import i18n from "@/i18n";
import { normalizeOverviewSnapshot } from "@/lib/normalizeUiSnapshots";
import { MOCK_OVERVIEW, type FiatCompleteness } from "@/mocks/seed";

import { buildStatsData } from "./model";
import { StatsCards } from "./StatsCards";
import { statStatusText } from "./statStatus";

const INCOMPLETE: FiatCompleteness = {
  state: "incomplete",
  costBasisComplete: false,
  reasons: ["quarantines"],
  quarantineCount: 2,
  quarantinedInboundMsat: 12_000_000_000,
  quarantinedOutboundMsat: 0,
  basisCoveredMsat: 426_000_000_000,
  basisUncoveredMsat: 0,
  earliestIncompleteAt: null,
  missingPriceCount: 0,
  marketRateMissing: false,
};

describe("overview stats cards", () => {
  it("does not label the BTC balance as an estimate", () => {
    const bitcoinBalanceStat = buildStatsData(MOCK_OVERVIEW, "btc")[0];
    const zeroBitcoinBalanceStat = { ...bitcoinBalanceStat, value: 0 };
    const fiatPortfolioStat = {
      ...buildStatsData(MOCK_OVERVIEW, "eur")[0],
      previousValue: 0,
    };

    expect(statStatusText(bitcoinBalanceStat, true)).toBe("Current");
    expect(statStatusText(zeroBitcoinBalanceStat, true)).toBe("Current");
    expect(statStatusText(fiatPortfolioStat, false)).toBe("Estimate");
  });

  it("keeps metric values visible during refresh", () => {
    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot: MOCK_OVERVIEW,
        hideSensitive: false,
        currency: "btc",
        isRefreshing: true,
        isMarketRateRefreshing: true,
      }),
    );

    expect(html).toContain("BTC price");
    expect(html).toContain("Bitcoin balance");
    expect(html).toContain("Refreshing");
    expect(html).not.toContain('data-slot="skeleton"');
  });

  it("does not call the main balance current while journals are quarantined", () => {
    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot: {
          ...MOCK_OVERVIEW,
          balanceSummary: {
            totalBtc: 1.236,
            status: "quarantines",
            source: "mixed",
            needsJournals: false,
            quarantines: 2,
            chainWalletCount: 1,
            bookWalletCount: 1,
            transactionWalletCount: 0,
            duplicateOutpointAdjustmentBtc: 0,
          },
        },
        hideSensitive: false,
        currency: "btc",
      }),
    );

    expect(html).toContain("2 quarantines");
    expect(html).toContain('href="/quarantine"');
    expect(html).not.toContain(">Current<");
  });

  it("formats overview counts with Austrian grouping", () => {
    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot: {
          ...MOCK_OVERVIEW,
          status: {
            ...MOCK_OVERVIEW.status!,
            transactionCount: 1_234,
          },
        },
        hideSensitive: false,
        currency: "eur",
        isRefreshing: false,
        isMarketRateRefreshing: false,
      }),
    );

    expect(html).toContain("1\u00a0234");
    expect(html).not.toContain("1,234");
  });

  it("marks the fiat value incomplete and names the BTC without cost basis", () => {
    const snapshot = normalizeOverviewSnapshot({
      ...MOCK_OVERVIEW,
      fiat: {
        ...MOCK_OVERVIEW.fiat,
        completeness: {
          ...INCOMPLETE,
          basisUncoveredMsat: 12_000_000_000,
          earliestIncompleteAt: "2026-02-01T00:00:00Z",
        },
      },
    });
    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot,
        hideSensitive: false,
        currency: "eur",
      }),
    );

    expect(html).toContain("Incomplete");
    expect(html).toContain("\u20bf 0.12000000 without cost basis");
    expect(html).toContain('href="/quarantine"');
    expect(html).toContain("text-amber-600");
    expect(html).not.toContain("vs cost basis");
    // The misleading "+57.6%" unrealized-vs-basis percentage is gone.
    expect(html).not.toMatch(/[+-]\d+\.\d%/);
  });

  it("calls a stale basis outdated and links to journals", () => {
    const snapshot = normalizeOverviewSnapshot({
      ...MOCK_OVERVIEW,
      fiat: {
        ...MOCK_OVERVIEW.fiat,
        completeness: {
          ...INCOMPLETE,
          state: "stale",
          reasons: ["journals_stale"],
          quarantineCount: 0,
          basisCoveredMsat: null,
          basisUncoveredMsat: null,
        },
      },
    });
    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot,
        hideSensitive: false,
        currency: "eur",
      }),
    );

    expect(html).toContain("Outdated");
    expect(html).toContain("Cost basis from outdated journals");
    expect(html).toContain('href="/journals"');
  });

  it("shows a dash instead of a zero value without a market rate", () => {
    const snapshot = normalizeOverviewSnapshot({
      ...MOCK_OVERVIEW,
      marketRate: { ...MOCK_OVERVIEW.marketRate, rate: null },
      fiat: {
        ...MOCK_OVERVIEW.fiat,
        eurBalance: 0,
        eurUnrealized: -198_502.4,
        completeness: {
          ...MOCK_OVERVIEW.fiat.completeness,
          state: "unavailable",
          reasons: ["market_rate_missing"],
          marketRateMissing: true,
        },
      },
    });
    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot,
        hideSensitive: false,
        currency: "eur",
      }),
    );

    expect(html).toContain("No market rate");
    expect(html).toContain("<span>\u2014</span>");
    expect(html).not.toContain("-100.0%");
    expect(html).not.toContain("\u20ac 0");
  });

  it("labels the incomplete fiat portfolio in Austrian German", async () => {
    await i18n.changeLanguage("de");
    try {
      const snapshot = normalizeOverviewSnapshot({
        ...MOCK_OVERVIEW,
        fiat: {
          ...MOCK_OVERVIEW.fiat,
          completeness: { ...INCOMPLETE, basisUncoveredMsat: 12_000_000_000 },
        },
      });
      const html = renderToStaticMarkup(
        createElement(StatsCards, {
          snapshot,
          hideSensitive: false,
          currency: "eur",
        }),
      );

      expect(html).toContain("Unvollst\u00e4ndig");
      expect(html).toContain("\u20bf 0.12000000 ohne Anschaffungskosten");
    } finally {
      await i18n.changeLanguage("en");
    }
  });

  it("reports the completeness badge through statStatusText", () => {
    const fiatPortfolioStat = buildStatsData(MOCK_OVERVIEW, "eur")[0];
    const bitcoinBalanceStat = buildStatsData(MOCK_OVERVIEW, "btc")[0];

    expect(statStatusText(fiatPortfolioStat, false, INCOMPLETE)).toBe("Incomplete");
    // The observed BTC balance is not basis-derived.
    expect(statStatusText(bitcoinBalanceStat, true, INCOMPLETE)).toBe("Current");
  });

  it("drops the change percentage when the basis is incomplete", () => {
    const stat = buildStatsData(
      { ...MOCK_OVERVIEW, fiat: { ...MOCK_OVERVIEW.fiat, completeness: INCOMPLETE } },
      "eur",
    )[0];

    expect(stat.changePercent).toBe(0);
  });
});
