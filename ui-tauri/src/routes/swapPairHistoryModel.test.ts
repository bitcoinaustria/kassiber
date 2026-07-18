import { describe, expect, it } from "vitest";

import {
  formatPairHistoryAssetTotals,
  formatPairHistoryMsatAsBtc,
  groupPairedComponents,
  pairHistoryFeePercent,
} from "./swapPairHistoryModel";

describe("paired transfer history model", () => {
  it("groups a many-to-one component and sums each allocation once", () => {
    const pairs = [
      {
        id: "pair-a",
        component_id: "component-1",
        component: { id: "component-1", source_count: 2, sink_count: 1 },
        out: { asset: "BTC", amount_msat: 60 },
        in: { asset: "BTC", amount_msat: 60 },
      },
      {
        id: "pair-b",
        component_id: "component-1",
        component: { id: "component-1", source_count: 2, sink_count: 1 },
        out: { asset: "BTC", amount_msat: 40 },
        in: { asset: "BTC", amount_msat: 40 },
      },
    ];

    expect(groupPairedComponents(pairs)).toEqual([
      {
        id: "component-1",
        sourceCount: 2,
        sinkCount: 1,
        pairs,
        sourceTotals: [{ asset: "BTC", amountMsat: 100n }],
        sinkTotals: [{ asset: "BTC", amountMsat: 100n }],
      },
    ]);
  });

  it("keeps one-to-many topology visible as one component", () => {
    const pairs = [
      {
        id: "pair-a",
        component_id: "component-1",
        component: { id: "component-1", source_count: 1, sink_count: 2 },
        out: { asset: "BTC", amount_msat: 70 },
        in: { asset: "BTC", amount_msat: 70 },
      },
      {
        id: "pair-b",
        component_id: "component-1",
        component: { id: "component-1", source_count: 1, sink_count: 2 },
        out: { asset: "BTC", amount_msat: 30 },
        in: { asset: "BTC", amount_msat: 30 },
      },
    ];

    const [component] = groupPairedComponents(pairs);

    expect(component.sourceCount).toBe(1);
    expect(component.sinkCount).toBe(2);
    expect(component.sourceTotals).toEqual([{ asset: "BTC", amountMsat: 100n }]);
    expect(component.sinkTotals).toEqual([{ asset: "BTC", amountMsat: 100n }]);
  });

  it("keeps unlike assets separate and labels each total", () => {
    const [component] = groupPairedComponents([
      {
        id: "pair-a",
        component_id: "component-1",
        component: { id: "component-1", source_count: 2, sink_count: 1 },
        out: { asset: "BTC", amount_msat: 60_000_000 },
        in: { asset: "ETH", amount_msat: 70_000_000 },
      },
      {
        id: "pair-b",
        component_id: "component-1",
        component: { id: "component-1", source_count: 2, sink_count: 1 },
        out: { asset: "LBTC", amount_msat: 40_000_000 },
        in: { asset: "ETH", amount_msat: 30_000_000 },
      },
    ]);

    expect(formatPairHistoryAssetTotals(component.sourceTotals)).toBe(
      "0.00060000 BTC + 0.00040000 LBTC",
    );
    expect(formatPairHistoryAssetTotals(component.sinkTotals)).toBe(
      "0.00100000 ETH",
    );
  });

  it("formats exact integer strings without Number precision loss", () => {
    expect(formatPairHistoryMsatAsBtc(9_007_199_254_740_993n)).toBe(
      "₿90071.99254741",
    );
    expect(
      pairHistoryFeePercent(
        "9007199254740993",
        "18014398509481986",
      ),
    ).toBe(50);
  });
});
