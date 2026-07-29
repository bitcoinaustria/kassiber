import { describe, expect, it } from "vitest";

import {
  activityFlowShares,
  activityFlowSlicePath,
  type ActivityFlow,
  type TreasuryChartPoint,
} from "./model";

function point(flow: ActivityFlow, eventSize: number): TreasuryChartPoint {
  return {
    date: "2026-01-01",
    month: "Jan",
    detailLabel: "Jan 1, 2026",
    thisYear: 0,
    balanceBtc: 1,
    valueEur: 0,
    costBasisEur: 0,
    unrealizedEur: 0,
    bitcoinPriceEur: 60_000,
    avgCostEur: 60_000,
    brushBalanceBtc: 1,
    reserveValueEur: 0,
    activityBtc: eventSize,
    activityCount: 1,
    activityValueEur: 0,
    eventSize,
    eventFlow: flow,
    sortTimeMs: 0,
    isActivityEvent: true,
  };
}

describe("activity marker dot", () => {
  it("weights flow slices by volume and always fills the circle", () => {
    const slices = activityFlowShares([
      point("incoming", 3),
      point("outgoing", 1),
      point("incoming", 1),
    ]);

    expect(slices.map((slice) => slice.flow)).toEqual(["incoming", "outgoing"]);
    expect(slices[0]?.share).toBeCloseTo(0.8, 10);
    expect(slices.reduce((sum, slice) => sum + slice.share, 0)).toBeCloseTo(1, 10);
  });

  it("keeps zero-volume events visible instead of collapsing the slice", () => {
    const slices = activityFlowShares([point("incoming", 0), point("outgoing", 0)]);

    expect(slices).toHaveLength(2);
    expect(slices[0]?.share).toBeCloseTo(0.5, 10);
  });

  it("draws a slice from twelve o'clock, flagging arcs over a half turn", () => {
    // Quarter turn: starts at the top, ends at 3 o'clock, small-arc flag.
    expect(activityFlowSlicePath(100, 50, 10, 0, 0.25)).toBe(
      "M 100 50 L 100 40 A 10 10 0 0 1 110 50 Z",
    );
    expect(activityFlowSlicePath(0, 0, 10, 0, 0.75)).toContain("A 10 10 0 1 1");
  });
});
