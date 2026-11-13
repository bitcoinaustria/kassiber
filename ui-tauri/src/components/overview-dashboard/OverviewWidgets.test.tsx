import { createElement } from "react";
import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import "@/i18n";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: ComponentProps<"a"> & { to?: string }) => <a href={to}>{children}</a>,
}));

import { MOCK_OVERVIEW, type FiatCompleteness, type OverviewSnapshot } from "@/mocks/seed";

import { monthlyFlows } from "./model";
import { FlowsWidget, TaxYearWidget } from "./OverviewWidgets";

const COMPLETE: FiatCompleteness = {
  state: "complete",
  costBasisComplete: true,
  reasons: [],
  quarantineCount: 0,
  quarantinedInboundMsat: 0,
  quarantinedOutboundMsat: 0,
  basisCoveredMsat: 100_000_000_000,
  basisUncoveredMsat: 0,
  earliestIncompleteAt: null,
  missingPriceCount: 0,
  marketRateMissing: false,
};

describe("tax year widget", () => {
  it("withholds gains while the basis is incomplete", () => {
    const html = renderToStaticMarkup(
      createElement(TaxYearWidget, {
        snapshot: {
          ...MOCK_OVERVIEW,
          fiat: { ...MOCK_OVERVIEW.fiat, eurRealizedYTD: 1234, completeness: { ...COMPLETE, costBasisComplete: false } },
        },
        hideSensitive: false,
      }),
    );
    expect(html).not.toContain("1.234");
    expect(html).toContain("Cost basis is incomplete");
  });

  it("adds the Austrian tax-free split when the book has one", () => {
    const html = renderToStaticMarkup(
      createElement(TaxYearWidget, {
        snapshot: {
          ...MOCK_OVERVIEW,
          fiat: { ...MOCK_OVERVIEW.fiat, completeness: COMPLETE },
          taxFreeBalance: {
            rule: "austrian_altbestand",
            jurisdictionCode: "AT",
            fiatCurrency: "EUR",
            status: "current",
            taxFreeQuantitySats: 50_000_000,
            taxableQuantitySats: 25_000_000,
            totalQuantitySats: 75_000_000,
            taxFreeMarketValue: null,
            taxableMarketValue: null,
            needsJournals: false,
            quarantines: 0,
            buckets: [],
          },
        },
        hideSensitive: false,
      }),
    );
    expect(html).toContain("Tax-free (old holdings)");
    expect(html).toContain("0.50000000");
  });
});

describe("tax year widget privacy", () => {
  it("masks its amounts while they are hidden", () => {
    const html = renderToStaticMarkup(
      createElement(TaxYearWidget, {
        snapshot: { ...MOCK_OVERVIEW, fiat: { ...MOCK_OVERVIEW.fiat, eurCostBasis: 4321, completeness: COMPLETE } },
        hideSensitive: true,
      }),
    );
    expect(html).toContain("••••");
    expect(html).not.toContain("4.321");
  });
});

describe("money in and out", () => {
  const NOW = new Date(2026, 10, 15);
  const tx = (id: string, date: string, amountSat: number, eur: number, extra: Partial<OverviewSnapshot["txs"][number]> = {}) => ({
    ...MOCK_OVERVIEW.txs[0],
    id,
    date,
    occurredAt: undefined,
    amountSat,
    eur,
    feeSat: 0,
    internal: false,
    type: amountSat >= 0 ? ("Income" as const) : ("Expense" as const),
    ...extra,
  });
  const snapshot = (txs: OverviewSnapshot["txs"]): OverviewSnapshot => ({ ...MOCK_OVERVIEW, activityTxs: txs, txs });

  it("sums received, spent and fees by month over the last twelve", () => {
    const flows = monthlyFlows(
      snapshot([
        tx("in", "2026-03-04 10:00", 50_000_000, 30_000),
        tx("out", "2026-11-02 10:00", -10_000_000, -6_000, { feeSat: 2_000 }),
        tx("move", "2026-06-01 10:00", -20_000_000, -12_000, { type: "Transfer", internal: true, feeSat: 1_000 }),
        tx("old", "2025-10-30 10:00", 90_000_000, 50_000),
      ]),
      NOW,
    );
    expect(flows.months).toHaveLength(12);
    expect(flows.months[0].date.getMonth()).toBe(11);
    expect(flows.inBtc).toBeCloseTo(0.5);
    expect(flows.outBtc).toBeCloseTo(0.1);
    expect(flows.inValue).toBe(30_000);
    // A move between own wallets adds only its fee.
    expect(flows.feeBtc).toBeCloseTo(0.00003);
    expect(flows.hasActivity).toBe(true);
  });

  it("is left out when the last twelve months saw nothing", () => {
    const quiet = snapshot([tx("old", "2024-01-10 10:00", 90_000_000, 50_000)]);
    expect(monthlyFlows(quiet, NOW).hasActivity).toBe(false);
    const html = renderToStaticMarkup(
      createElement(FlowsWidget, { snapshot: quiet, hideSensitive: false, currency: "eur" }),
    );
    expect(html).toBe("");
  });
});
