import { describe, expect, it } from "vitest";

import {
  normalizeOverviewSnapshot,
  normalizeQuarantineSnapshot,
} from "./normalizeUiSnapshots";

describe("normalizeOverviewSnapshot", () => {
  it("turns missing array fields into empty arrays", () => {
    const snapshot = normalizeOverviewSnapshot({
      fiat: {},
      status: { needsJournals: false, quarantines: 0 },
    });

    expect(snapshot.connections).toEqual([]);
    expect(snapshot.activityTxs).toEqual([]);
    expect(snapshot.txs).toEqual([]);
    expect(snapshot.balanceSeries).toEqual([]);
    expect(snapshot.portfolioSeries).toBeUndefined();
  });

  it("drops non-finite balance-series values", () => {
    const snapshot = normalizeOverviewSnapshot({
      balanceSeries: [1, null, "2", Number.NaN, 3],
      fiat: {},
    });

    expect(snapshot.balanceSeries).toEqual([1, 3]);
  });
});

describe("normalizeQuarantineSnapshot", () => {
  it("provides safe summary and item arrays for partial responses", () => {
    const snapshot = normalizeQuarantineSnapshot({ summary: {} });

    expect(snapshot.summary.by_reason).toEqual([]);
    expect(snapshot.items).toEqual([]);
    expect(snapshot.summary.count).toBe(0);
  });
});
