import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { TreasuryTooltip, type TreasuryTooltipPayload } from "./TreasuryTooltip";
import {
  createHoveredActivityPointStore,
  powerLawDaysFor,
  type TreasuryChartPoint,
} from "./model";

function activityPoint(
  id: string,
  signedBtc: number,
  balanceBtc: number,
): TreasuryChartPoint {
  return {
    date: `2026-05-15T12:00:00Z#${id}`,
    month: "May 15",
    detailLabel: "May 15, 2026",
    thisYear: balanceBtc * 65_000,
    prevYear: balanceBtc * 50_000,
    balanceBtc,
    valueEur: balanceBtc * 65_000,
    costBasisEur: balanceBtc * 50_000,
    unrealizedEur: balanceBtc * 15_000,
    bitcoinPriceEur: 65_000,
    avgCostEur: 50_000,
    lineBalanceBtc: undefined,
    lineBitcoinPriceEur: undefined,
    lineAvgCostEur: undefined,
    brushBalanceBtc: balanceBtc,
    reserveValueEur: balanceBtc * 65_000,
    activityBtc: Math.abs(signedBtc),
    activityCount: 1,
    activityValueEur: Math.abs(signedBtc) * 65_000,
    eventPriceEur: 65_000,
    eventBalanceBtc: balanceBtc,
    markerBalanceBtc: balanceBtc,
    eventSize: Math.abs(signedBtc),
    eventFlow: "incoming",
    eventSignedBtc: signedBtc,
    eventFeeBtc: 0,
    eventFiatValueEur: Math.abs(signedBtc) * 65_000,
    eventType: "Income",
    eventAccount: "Treasury",
    eventCounter: "Customer",
    eventTag: "Revenue",
    eventStatus: "confirmed",
    eventConfirmations: 6,
    eventId: id,
    eventTransactionId: id,
    sortTimeMs: Date.parse("2026-05-15T12:00:00Z"),
    powerLawDays: powerLawDaysFor(Date.parse("2026-05-15T12:00:00Z")),
    isActivityEvent: true,
  };
}

describe("treasury tooltip", () => {
  it("prefers the hovered marker point over the shared chart payload", () => {
    const stalePayloadPoint = activityPoint("tx-stale", 0.21, 1.2);
    const hoveredPoint = activityPoint("tx-hovered", 0.05, 1.05);
    const hoverStore = createHoveredActivityPointStore();
    hoverStore.set(hoveredPoint);
    const payload: TreasuryTooltipPayload[] = [
      { dataKey: "markerBalanceBtc", payload: stalePayloadPoint },
    ];

    const html = renderToStaticMarkup(
      <TreasuryTooltip
        active
        hoveredPointStore={hoverStore}
        fiatCurrency="EUR"
        hideSensitive={false}
        payload={payload}
        priceEur={65_000}
      />,
    );

    expect(html).toContain("0.05000000");
    expect(html).toContain("tx-hovered");
    expect(html).not.toContain("0.21000000");
    expect(html).not.toContain("tx-stale");
  });

  it("dashes avg cost and unrealized at points past the first basis gap", () => {
    const point: TreasuryChartPoint = {
      ...activityPoint("day", 0, 1.2),
      isActivityEvent: false,
      eventFlow: undefined,
      basisIncomplete: true,
      avgCostEur: null,
    };

    const html = renderToStaticMarkup(
      <TreasuryTooltip
        active
        fiatCurrency="EUR"
        hideSensitive={false}
        payload={[{ dataKey: "lineBalanceBtc", payload: point }]}
        priceEur={65_000}
      />,
    );

    expect(html).toContain("Avg cost / BTC");
    expect(html).toContain("Unrealized");
    expect(html).not.toContain("18.000");
    expect(html).not.toContain("(+");
    expect(html).toContain("Cost basis is incomplete here");
    expect(html.match(/—/g)?.length).toBeGreaterThanOrEqual(2);
  });

  it("keeps avg cost and unrealized for points before the gap", () => {
    const point: TreasuryChartPoint = {
      ...activityPoint("day", 0, 1.2),
      isActivityEvent: false,
      eventFlow: undefined,
    };

    const html = renderToStaticMarkup(
      <TreasuryTooltip
        active
        fiatCurrency="EUR"
        hideSensitive={false}
        payload={[{ dataKey: "lineBalanceBtc", payload: point }]}
        priceEur={65_000}
      />,
    );

    expect(html).toContain("(+30.0%)");
    expect(html).not.toContain("Cost basis is incomplete here");
  });
});
