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

  it("keeps the daemon balance summary so wallets do not double count", () => {
    const snapshot = normalizeOverviewSnapshot({
      fiat: {},
      connections: [
        { id: "a", balance: 1 },
        { id: "b", balance: 1 },
      ],
      balanceSummary: {
        totalBtc: 1,
        status: "quarantines",
        source: "chain",
        needsJournals: false,
        quarantines: 2,
        chainWalletCount: 2,
        bookWalletCount: 0,
        transactionWalletCount: 0,
        duplicateOutpointAdjustmentBtc: 1,
      },
    });

    expect(snapshot.balanceSummary?.totalBtc).toBe(1);
    expect(snapshot.balanceSummary?.quarantines).toBe(2);
    expect(snapshot.balanceSummary?.duplicateOutpointAdjustmentBtc).toBe(1);
  });

  it("omits a balance summary without a finite total", () => {
    const snapshot = normalizeOverviewSnapshot({
      fiat: {},
      balanceSummary: { totalBtc: "1" },
    });

    expect(snapshot.balanceSummary).toBeUndefined();
  });

  it("passes fiat completeness through", () => {
    const snapshot = normalizeOverviewSnapshot({
      fiat: {
        completeness: {
          state: "incomplete",
          costBasisComplete: false,
          reasons: ["quarantines", "unknown_reason"],
          quarantineCount: 1,
          quarantinedInboundMsat: 12_000_000_000,
          quarantinedOutboundMsat: 0,
          basisCoveredMsat: 75_000_000_000,
          basisUncoveredMsat: 12_000_000_000,
          earliestIncompleteAt: "2026-06-04T08:00:00Z",
          missingPriceCount: 0,
          marketRateMissing: false,
        },
      },
    });

    expect(snapshot.fiat.completeness).toEqual({
      state: "incomplete",
      costBasisComplete: false,
      reasons: ["quarantines"],
      quarantineCount: 1,
      quarantinedInboundMsat: 12_000_000_000,
      quarantinedOutboundMsat: 0,
      basisCoveredMsat: 75_000_000_000,
      basisUncoveredMsat: 12_000_000_000,
      earliestIncompleteAt: "2026-06-04T08:00:00Z",
      missingPriceCount: 0,
      marketRateMissing: false,
    });
  });

  it("never treats a missing or contradictory completeness block as complete", () => {
    const legacy = normalizeOverviewSnapshot({ fiat: { eurCostBasis: 10 } });
    expect(legacy.fiat.completeness?.state).toBe("unavailable");
    expect(legacy.fiat.completeness?.costBasisComplete).toBe(false);

    const contradictory = normalizeOverviewSnapshot({
      fiat: { completeness: { state: "stale", costBasisComplete: true } },
    });
    expect(contradictory.fiat.completeness?.costBasisComplete).toBe(false);
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
