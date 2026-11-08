import { createElement } from "react";
import type { ComponentProps } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import "@/i18n";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: ComponentProps<"a"> & { to?: string }) => <a href={to}>{children}</a>,
}));

import { MOCK_OVERVIEW, type FiatCompleteness } from "@/mocks/seed";

import { TaxYearWidget } from "./OverviewWidgets";

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
