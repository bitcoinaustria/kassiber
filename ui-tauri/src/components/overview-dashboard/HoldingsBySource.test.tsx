import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
import "@/i18n";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: ComponentProps<"a">) => <a>{children}</a>,
}));

import { MOCK_OVERVIEW, type FiatCompleteness } from "@/mocks/seed";

import { DEFAULT_HOLDINGS_VIEW, normalizeHoldingsView, type HoldingsView } from "@/store/ui";

import { HoldingsBySourceChart } from "./HoldingsBySource";
import {
  HoldingsListLayout,
  HoldingsTableLayout,
  HoldingsTilesLayout,
  HoldingsTrendLayout,
} from "./HoldingsLayouts";
import { buildHoldingsBySource, monthEndTrend } from "./model";

const UNCOVERED: FiatCompleteness = {
  state: "incomplete",
  costBasisComplete: false,
  reasons: ["quarantines"],
  quarantineCount: 1,
  quarantinedInboundMsat: 12_000_000_000,
  quarantinedOutboundMsat: 0,
  basisCoveredMsat: 100_000_000_000,
  basisUncoveredMsat: 12_000_000_000,
  earliestIncompleteAt: "2026-01-03T10:00:00Z",
  missingPriceCount: 0,
  marketRateMissing: false,
};

describe("holdings header", () => {
  const render = (snapshot: typeof MOCK_OVERVIEW, currency: "btc" | "eur" = "eur") =>
    renderToStaticMarkup(
      createElement(HoldingsBySourceChart, { snapshot, hideSensitive: false, currency }),
    );

  it("leaves an incomplete basis to the tile and the chart", () => {
    const html = render({ ...MOCK_OVERVIEW, fiat: { ...MOCK_OVERVIEW.fiat, completeness: UNCOVERED } });
    expect(html).not.toContain("without cost basis");
    expect(html).not.toContain("vs cost basis");
  });

  it("ranks and sizes sources by the bitcoin held when there is no rate", () => {
    const connections = [0.5, 2].map((balance, index) => ({
      ...MOCK_OVERVIEW.connections[0],
      id: `wallet-${index}`,
      label: `Wallet ${index}`,
      balance,
    }));
    const html = render(
      {
        ...MOCK_OVERVIEW,
        connections,
        priceEur: 0,
        priceUsd: 0,
        marketRate: { ...MOCK_OVERVIEW.marketRate!, rate: null },
      },
      "btc",
    );
    expect(html.indexOf("Wallet 1")).toBeLessThan(html.indexOf("Wallet 0"));
    expect(html).toContain("80%");
    expect(html).toContain("20%");
    expect(html).toMatch(/2\.500/);
  });

  it("says so when no wallet holds bitcoin", () => {
    expect(render({ ...MOCK_OVERVIEW, connections: [] })).toContain("No wallet holds bitcoin yet.");
  });
});

describe("holdings by source", () => {
  it("lists each source with its share and folds the smallest into Other sources", () => {
    const connections = Array.from({ length: 10 }, (_, index) => ({
      ...MOCK_OVERVIEW.connections[0],
      id: `wallet-${index}`,
      label: `Wallet ${index}`,
      balance: 10 - index,
    }));
    const html = renderToStaticMarkup(
      createElement(HoldingsBySourceChart, {
        snapshot: { ...MOCK_OVERVIEW, connections },
        hideSensitive: false,
        currency: "btc",
      }),
    );
    for (const index of [0, 6]) expect(html).toContain(`Wallet ${index}`);
    expect(html).not.toContain("Wallet 7");
    expect(html).toContain("Other sources");
  });

  it("tells sources apart in whole euros and marks a sliver as under 1%", () => {
    const rate = 50_000;
    const connections = [40.2, 40.0, 0.001].map((balance, index) => ({
      ...MOCK_OVERVIEW.connections[0],
      id: `wallet-${index}`,
      label: `Wallet ${index}`,
      balance,
    }));
    const html = renderToStaticMarkup(
      createElement(HoldingsBySourceChart, {
        snapshot: {
          ...MOCK_OVERVIEW,
          connections,
          marketRate: { ...MOCK_OVERVIEW.marketRate!, rate, fiatCurrency: "EUR" },
        },
        hideSensitive: false,
        currency: "eur",
      }),
    ).replace(/\u00a0|\u202f/g, " ");
    expect(html).toContain("€ 2.010.000");
    expect(html).toContain("€ 2.000.000");
    expect(html).toContain("&lt;1%");
  });

  it("names when each source last synced, from the snapshot alone", () => {
    const connections = [
      { ...MOCK_OVERVIEW.connections[0], id: "synced", label: "Synced wallet", balance: 2, lastSyncAt: "2026-06-26T11:58:03Z" },
      { ...MOCK_OVERVIEW.connections[0], id: "import", label: "CSV import", balance: 1, lastSyncAt: null },
    ];
    const html = renderToStaticMarkup(
      createElement(HoldingsBySourceChart, {
        snapshot: { ...MOCK_OVERVIEW, connections },
        hideSensitive: false,
        currency: "btc",
      }),
    );
    expect(html.match(/Last synced/g)).toHaveLength(1);
    expect(html).toMatch(/synced \d+d ago/);
  });
});

describe("holdings view settings", () => {
  const wallet = (label: string, balance: number, extra: Partial<(typeof MOCK_OVERVIEW.connections)[number]> = {}) => ({
    ...MOCK_OVERVIEW.connections[0],
    kind: "xpub" as const,
    chain: "bitcoin",
    id: label,
    label,
    balance,
    lastTransactionAt: "2026-01-01T00:00:00Z",
    ...extra,
  });
  const snapshot = {
    ...MOCK_OVERVIEW,
    connections: [
      wallet("Cold", 2, { lastTransactionAt: "2025-03-01T00:00:00Z" }),
      wallet("Alpha Liquid", 1, { chain: "liquid", lastTransactionAt: "2026-06-01T00:00:00Z" }),
      wallet("Emptied", 0),
      wallet("Never funded", 0, { lastTransactionAt: null }),
    ],
  };
  const names = (view: Partial<HoldingsView> = {}) => {
    const result = buildHoldingsBySource(snapshot, { ...DEFAULT_HOLDINGS_VIEW, ...view });
    return {
      listed: result.items.map((item) => item.name),
      folded: result.folded.map((item) => item.name),
      filteredOut: result.filteredOut,
      totalBtc: result.totalBtc,
    };
  };

  it("folds an emptied source by default and never lists one without history", () => {
    expect(names()).toMatchObject({ listed: ["Cold", "Alpha Liquid"], folded: ["Emptied"] });
    expect(names({ empty: "show" }).listed).toEqual(["Cold", "Alpha Liquid", "Emptied"]);
    expect(names({ empty: "hide" })).toMatchObject({ listed: ["Cold", "Alpha Liquid"], folded: [] });
  });

  it("sorts by name or by the latest activity", () => {
    expect(names({ sort: "name" }).listed).toEqual(["Alpha Liquid", "Cold"]);
    expect(names({ sort: "activity" }).listed).toEqual(["Alpha Liquid", "Cold"]);
  });

  it("filters by rail and keeps each share of the whole book", () => {
    const result = buildHoldingsBySource(snapshot, { ...DEFAULT_HOLDINGS_VIEW, rails: ["liquid"] });
    expect(result.items.map((item) => [item.name, item.percent])).toEqual([["Alpha Liquid", 33]]);
    expect(result.totalBtc).toBe(1);
    expect(result.filteredOut).toBe(3);
  });

  it("restores every rail rather than an empty filter", () => {
    expect(normalizeHoldingsView({ rails: [], sort: "bogus", limit: 3 })).toEqual(DEFAULT_HOLDINGS_VIEW);
    // Layouts tried and dropped fall back to the list.
    expect(normalizeHoldingsView({ layout: "grouped" }).layout).toBe("list");
  });
});

describe("month-end trend", () => {
  const now = new Date(2026, 1, 15);

  it("carries each month-end balance forward across the last twelve months", () => {
    const trend = monthEndTrend(
      [
        { month: "2024-05", balanceBtc: 3 },
        { month: "2025-06", balanceBtc: 2.5 },
        { month: "2026-01", balanceBtc: 1 },
      ],
      now,
    );
    // March 2025 through February 2026, across the year boundary.
    expect(trend).toEqual([3, 3, 3, 2.5, 2.5, 2.5, 2.5, 2.5, 2.5, 2.5, 1, 1]);
  });

  it("starts at zero before the first month the wallet moved", () => {
    expect(monthEndTrend([{ month: "2026-02", balanceBtc: 0.25 }], now, 3)).toEqual([0, 0, 0.25]);
  });

  it("has no points for a wallet that never moved", () => {
    expect(monthEndTrend([], now)).toEqual([]);
  });
});

describe("holdings layouts", () => {
  const connections = [
    { ...MOCK_OVERVIEW.connections[0], id: "cold", label: "Cold", kind: "xpub" as const, chain: "bitcoin", balance: 2, lastTransactionAt: "2026-01-01", transactionCount: 4,
      monthBalances: [{ month: "2025-12", balanceBtc: 1 }, { month: "2026-01", balanceBtc: 2 }] },
    { ...MOCK_OVERVIEW.connections[0], id: "liq", label: "Liquid", kind: "address" as const, chain: "liquid", balance: 1, lastTransactionAt: "2026-02-01", transactionCount: 2 },
  ];
  const props = () => ({
    rows: buildHoldingsBySource(
      { ...MOCK_OVERVIEW, connections },
      { ...DEFAULT_HOLDINGS_VIEW, limit: 0 },
      new Date(2026, 10, 7),
    ).items,
    hideSensitive: false,
    hideShares: false,
    valueLabel: (btc: number) => `₿ ${btc.toFixed(3)}`,
    fiatLabel: (value: number) => `€ ${Math.round(value)}`,
    isBitcoinMode: true,
  });

  it("gives each tile its other unit and transaction count", () => {
    const html = renderToStaticMarkup(createElement(HoldingsTilesLayout, props()));
    expect(html).toMatch(/€ \d/);
    expect(html).toContain("4 transactions");
  });

  it("puts exact bitcoin and transaction counts in the table", () => {
    const html = renderToStaticMarkup(createElement(HoldingsTableLayout, props()));
    expect(html).toContain("₿ 2.00000000");
    expect(html).toContain(">4<");
  });

  it("keeps trend rows in one column until two of them fit", () => {
    const trend = renderToStaticMarkup(createElement(HoldingsTrendLayout, props()));
    expect(trend).toContain("@min-[72rem]/holdings:grid-cols-2");
    expect(trend).not.toContain("@min-[44rem]/holdings:grid-cols-2");
    const list = renderToStaticMarkup(createElement(HoldingsListLayout, props()));
    expect(list).toContain("@min-[44rem]/holdings:grid-cols-2");
  });

  it("masks shares and its own amounts in every layout under Hide all", () => {
    for (const layout of [HoldingsListLayout, HoldingsTilesLayout, HoldingsTableLayout, HoldingsTrendLayout]) {
      const html = renderToStaticMarkup(
        createElement(layout, { ...props(), hideSensitive: true, hideShares: true }),
      );
      expect(html).not.toMatch(/>(&lt;)?\d+%</);
      expect(html).not.toContain("2.00000000");
      expect(html).not.toContain("+ ₿ 1.000");
    }
  });

  it("shows the twelve-month change beside the trend", () => {
    const html = renderToStaticMarkup(createElement(HoldingsTrendLayout, props()));
    expect(html).toContain("+ ₿ 1.000");
    expect(html).toContain("<polyline");
  });
});
