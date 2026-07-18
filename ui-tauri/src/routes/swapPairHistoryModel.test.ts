import { describe, expect, it } from "vitest";

import {
  formatPairHistoryMsatAsBtc,
  groupPairedComponents,
} from "./swapPairHistoryModel";

describe("paired transfer history model", () => {
  it("groups a many-to-one component and sums each allocation once", () => {
    const pairs = [
      {
        id: "pair-a",
        component_id: "component-1",
        component: { id: "component-1", source_count: 2, sink_count: 1 },
        out: { amount_msat: 60 },
        in: { amount_msat: 60 },
      },
      {
        id: "pair-b",
        component_id: "component-1",
        component: { id: "component-1", source_count: 2, sink_count: 1 },
        out: { amount_msat: 40 },
        in: { amount_msat: 40 },
      },
    ];

    expect(groupPairedComponents(pairs)).toEqual([
      {
        id: "component-1",
        sourceCount: 2,
        sinkCount: 1,
        pairs,
        sourceTotalMsat: 100n,
        sinkTotalMsat: 100n,
      },
    ]);
  });

  it("keeps one-to-many topology visible as one component", () => {
    const pairs = [
      {
        id: "pair-a",
        component_id: "component-1",
        component: { id: "component-1", source_count: 1, sink_count: 2 },
        out: { amount_msat: 70 },
        in: { amount_msat: 70 },
      },
      {
        id: "pair-b",
        component_id: "component-1",
        component: { id: "component-1", source_count: 1, sink_count: 2 },
        out: { amount_msat: 30 },
        in: { amount_msat: 30 },
      },
    ];

    const [component] = groupPairedComponents(pairs);

    expect(component.sourceCount).toBe(1);
    expect(component.sinkCount).toBe(2);
    expect(component.sourceTotalMsat).toBe(100n);
    expect(component.sinkTotalMsat).toBe(100n);
  });

  it("formats exact integer strings without Number precision loss", () => {
    expect(formatPairHistoryMsatAsBtc(9_007_199_254_740_993n)).toBe(
      "₿90071.99254741",
    );
  });
});
