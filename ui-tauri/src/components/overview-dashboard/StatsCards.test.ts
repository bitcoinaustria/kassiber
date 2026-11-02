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

import { buildPortfolioStat } from "./model";
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
  it("calls a portfolio value without a basis an estimate", () => {
    const fiatPortfolioStat = { ...buildPortfolioStat(MOCK_OVERVIEW), previousValue: 0 };

    expect(statStatusText(fiatPortfolioStat)).toBe("Estimate");
  });

  it("shows the price, the bitcoin held and what it is worth, and nothing else", () => {
    const html = renderToStaticMarkup(
      createElement(StatsCards, { snapshot: MOCK_OVERVIEW, hideSensitive: false }),
    );

    expect(html).toContain("BTC price");
    expect(html).toContain("Bitcoin balance");
    expect(html).toContain("Portfolio value");
    expect(html).not.toContain(">Transactions<");
    expect(html).not.toContain(">Connections<");
    expect(html).not.toContain(">Open review<");
  });

  it("keeps metric values visible during refresh", () => {
    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot: MOCK_OVERVIEW,
        hideSensitive: false,
        isRefreshing: true,
        isMarketRateRefreshing: true,
      }),
    );

    expect(html).toContain("BTC price");
    expect(html).toContain("Bitcoin balance");
    expect(html).toContain("Refreshing");
    expect(html).not.toContain('data-slot="skeleton"');
  });

  it("puts quarantines on the portfolio value and gives the observed balance its wallets", () => {
    const html = renderToStaticMarkup(
      createElement(StatsCards, {
        snapshot: {
          ...MOCK_OVERVIEW,
          connections: [
            { ...MOCK_OVERVIEW.connections[0], id: "a", balance: 1 },
            { ...MOCK_OVERVIEW.connections[0], id: "b", balance: 0.236 },
            { ...MOCK_OVERVIEW.connections[0], id: "c", balance: 0 },
          ],
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
      }),
    );

    expect(html.match(/2 quarantines/g)).toHaveLength(1);
    expect(html).toContain('href="/review"');
    expect(html).toContain("in 2 wallets");
    expect(html).toContain('href="/connections"');
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
      }),
    );

    expect(html).toContain("Incomplete");
    expect(html).toContain("\u20bf 0.12000000 without cost basis");
    expect(html).toContain('href="/review"');
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
        }),
      );

      expect(html).toContain("Unvollst\u00e4ndig");
      expect(html).toContain("\u20bf 0.12000000 ohne Anschaffungskosten");
    } finally {
      await i18n.changeLanguage("en");
    }
  });

  it("reports the completeness badge through statStatusText", () => {
    const fiatPortfolioStat = buildPortfolioStat(MOCK_OVERVIEW);

    expect(statStatusText(fiatPortfolioStat, INCOMPLETE)).toBe("Incomplete");
  });

  it("drops the change percentage when the basis is incomplete", () => {
    const stat = buildPortfolioStat({
      ...MOCK_OVERVIEW,
      fiat: { ...MOCK_OVERVIEW.fiat, completeness: INCOMPLETE },
    });

    expect(stat.changePercent).toBe(0);
  });
});
