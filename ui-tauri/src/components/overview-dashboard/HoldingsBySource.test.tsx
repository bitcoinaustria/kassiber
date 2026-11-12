import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { ComponentProps } from "react";
import { describe, expect, it, vi } from "vitest";
import "@/i18n";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: ComponentProps<"a">) => <a>{children}</a>,
}));

import { MOCK_OVERVIEW, type FiatCompleteness } from "@/mocks/seed";

import { HoldingsBySourceChart } from "./HoldingsBySource";

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
