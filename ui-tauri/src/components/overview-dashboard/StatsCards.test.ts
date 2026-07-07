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

import { MOCK_OVERVIEW } from "@/mocks/seed";
import type { OverviewSnapshot } from "@/mocks/seed";

import { buildStatsData } from "./model";
import { StatsCards, statStatusText } from "./StatsCards";

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

  it("renders tax-free balance with daemon-provided bucket labels", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      taxFreeBalance: {
        rule: "generic_long_term_holding",
        jurisdictionCode: "EX",
        fiatCurrency: "EUR",
        status: "current",
        taxFreeQuantitySats: 210_000_000,
        taxableQuantitySats: 30_000_000,
        totalQuantitySats: 240_000_000,
        taxFreeMarketValue: 126_000,
        taxableMarketValue: 18_000,
        needsJournals: false,
        quarantines: 0,
        buckets: [
          {
            id: "long-term",
            regime: "long",
            label: "Long-term",
            quantitySats: 210_000_000,
            marketValue: 126_000,
            taxFree: true,
          },
          {
            id: "short-term",
            regime: "short",
            label: "Short-term",
            quantitySats: 30_000_000,
            marketValue: 18_000,
            taxFree: false,
          },
        ],
      },
    };

    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot,
        hideSensitive: false,
        currency: "btc",
      }),
    );

    expect(html).toContain("Tax-free balance");
    expect(html).toContain("\u20bf 2.100");
    expect(html).toContain("Long-term \u20bf 2.100 \u00b7 Short-term \u20bf 0.300");
  });

  it("blocks stale tax-free balances behind journal readiness", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      taxFreeBalance: {
        ...MOCK_OVERVIEW.taxFreeBalance!,
        status: "needs_journals",
        needsJournals: true,
      },
    };

    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot,
        hideSensitive: false,
        currency: "btc",
      }),
    );

    expect(html).toContain("Needs journals");
    expect(html).toContain("Review required");
    expect(html).toContain("Run journals before relying on this balance");
    expect(html).not.toContain("Altbestand \u20bf 1.200");
  });

  it("renders the tax-free balance card when the current amount is zero", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      taxFreeBalance: {
        ...MOCK_OVERVIEW.taxFreeBalance!,
        taxFreeQuantitySats: 0,
        taxableQuantitySats: 0,
        totalQuantitySats: 0,
        taxFreeMarketValue: 0,
        taxableMarketValue: 0,
        buckets: MOCK_OVERVIEW.taxFreeBalance!.buckets.map((bucket) => ({
          ...bucket,
          quantitySats: 0,
          marketValue: 0,
        })),
      },
    };

    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot,
        hideSensitive: false,
        currency: "btc",
      }),
    );

    expect(html).toContain("Tax-free balance");
    expect(html).toContain("\u20bf 0.000");
    expect(html).toContain("Altbestand \u20bf 0.000 \u00b7 Neubestand \u20bf 0.000");
  });

  it("blocks quarantined tax-free balances behind review", () => {
    const snapshot: OverviewSnapshot = {
      ...MOCK_OVERVIEW,
      taxFreeBalance: {
        ...MOCK_OVERVIEW.taxFreeBalance!,
        status: "quarantines",
        quarantines: 1,
      },
    };

    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot,
        hideSensitive: false,
        currency: "btc",
      }),
    );

    expect(html).toContain("1 quarantine");
    expect(html).toContain("Review required");
    expect(html).toContain("Review quarantines before relying on this balance");
    expect(html).not.toContain("Altbestand \u20bf 1.200");
  });
});
