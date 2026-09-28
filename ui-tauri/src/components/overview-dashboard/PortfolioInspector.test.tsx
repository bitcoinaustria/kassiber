import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { PortfolioChartPoint } from "./model";
import { PortfolioInspector } from "./PortfolioInspector";

const POINT: PortfolioChartPoint = {
  date: "2026-05-15",
  month: "May 15",
  detailLabel: "May 15, 2026",
  thisYear: 78_000,
  prevYear: 60_000,
  balanceBtc: 1.2,
  valueEur: 78_000,
  costBasisEur: 60_000,
  priceEur: 65_000,
  unrealizedEur: 18_000,
};

describe("portfolio inspector", () => {
  it.each(["panel", "header"] as const)(
    "shows cost basis and unrealized as a dash with a reason when incomplete (%s)",
    (variant) => {
      const html = renderToStaticMarkup(
        <PortfolioInspector
          point={{ ...POINT, basisIncomplete: true }}
          hideSensitive={false}
          priceEur={65_000}
          fiatCurrency="EUR"
          variant={variant}
        />,
      );

      expect(html).toContain("Total basis");
      expect(html).not.toContain("60.000");
      expect(html).not.toContain("18.000");
      expect(html.match(/>—</g)?.length).toBe(2);
      expect(html).toContain('title="Cost basis is incomplete here');
    },
  );

  it("uses the stale hint when journals are outdated", () => {
    const html = renderToStaticMarkup(
      <PortfolioInspector
        point={{ ...POINT, basisIncomplete: true }}
        hideSensitive={false}
        priceEur={65_000}
        fiatCurrency="EUR"
        basisHintKey="completeness.hint.stale"
      />,
    );

    expect(html).toContain("Journals are outdated");
  });

  it("keeps exact figures for points before the first gap", () => {
    const html = renderToStaticMarkup(
      <PortfolioInspector
        point={POINT}
        hideSensitive={false}
        priceEur={65_000}
        fiatCurrency="EUR"
      />,
    );

    expect(html).toContain("60.000");
    expect(html).toContain("18.000");
    expect(html).not.toContain(">—<");
  });
});
