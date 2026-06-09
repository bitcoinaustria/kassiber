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
});
