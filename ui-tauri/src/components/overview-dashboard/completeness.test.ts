import { describe, expect, it } from "vitest";

import {
  MOCK_OVERVIEW,
  type FiatCompleteness,
  type OverviewSnapshot,
} from "@/mocks/seed";

import {
  buildOverviewHealthItems,
  buildOverviewReadiness,
  buildPrimaryOverviewAction,
  enrichTreasuryChartData,
  getDataForPeriod,
  portfolioCompletenessDetail,
} from "./model";

const QUARANTINE_FROM_JAN_3: FiatCompleteness = {
  state: "incomplete",
  costBasisComplete: false,
  reasons: ["quarantines"],
  quarantineCount: 1,
  quarantinedInboundMsat: 10_000_000_000,
  quarantinedOutboundMsat: 0,
  basisCoveredMsat: 100_000_000_000,
  basisUncoveredMsat: 10_000_000_000,
  earliestIncompleteAt: "2026-01-03T10:00:00Z",
  missingPriceCount: 0,
  marketRateMissing: false,
};

function snapshotWith(completeness: FiatCompleteness): OverviewSnapshot {
  return {
    ...MOCK_OVERVIEW,
    activityTxs: [],
    txs: [],
    portfolioSeries: ["2026-01-01", "2026-01-02", "2026-01-03", "2026-01-04"].map(
      (date) => ({
        date,
        label: date,
        balanceBtc: 1,
        valueEur: 100_000,
        costBasisEur: 80_000,
        priceEur: 100_000,
      }),
    ),
    fiat: { ...MOCK_OVERVIEW.fiat, completeness },
    status: { ...MOCK_OVERVIEW.status!, quarantines: 0, needsJournals: false },
  };
}

describe("overview chart basis completeness", () => {
  it("keeps earlier points and blanks basis from the first gap day on", () => {
    const snapshot = snapshotWith(QUARANTINE_FROM_JAN_3);
    const points = enrichTreasuryChartData(
      getDataForPeriod("all", snapshot, "value", "eur", "detailed"),
      snapshot,
      "all",
    );
    const byDate = new Map(points.map((point) => [point.date, point]));

    expect(byDate.get("2026-01-02")?.lineAvgCostEur).toBe(80_000);
    expect(byDate.get("2026-01-02")?.basisIncomplete).toBeUndefined();
    expect(byDate.get("2026-01-03")?.basisIncomplete).toBe(true);
    expect(byDate.get("2026-01-03")?.lineAvgCostEur).toBeNull();
    expect(byDate.get("2026-01-04")?.avgCostEur).toBeNull();
  });

  it("treats an incomplete basis without a start date as affecting every point", () => {
    const snapshot = snapshotWith({
      ...QUARANTINE_FROM_JAN_3,
      state: "stale",
      reasons: ["journals_stale"],
      earliestIncompleteAt: null,
    });
    const points = getDataForPeriod("all", snapshot, "value", "eur", "detailed");

    expect(points.every((point) => point.basisIncomplete)).toBe(true);
  });

  it("leaves a complete basis untouched", () => {
    const snapshot = snapshotWith(MOCK_OVERVIEW.fiat.completeness!);
    const points = getDataForPeriod("all", snapshot, "value", "eur", "detailed");

    expect(points.some((point) => point.basisIncomplete)).toBe(false);
    expect(portfolioCompletenessDetail(snapshot)).toBeNull();
  });

  it("marks activity markers after the gap", () => {
    const snapshot: OverviewSnapshot = {
      ...snapshotWith(QUARANTINE_FROM_JAN_3),
      activityTxs: [
        {
          ...MOCK_OVERVIEW.txs[0],
          id: "before",
          occurredAt: "2026-01-02T12:00:00Z",
          amountSat: 1_000_000,
          balanceBtc: 1,
          costBasisEur: 80_000,
          excluded: false,
        },
        {
          ...MOCK_OVERVIEW.txs[0],
          id: "after",
          occurredAt: "2026-01-03T12:00:00Z",
          amountSat: 1_000_000,
          balanceBtc: 1.1,
          costBasisEur: 80_000,
          excluded: false,
        },
      ],
    };
    const points = enrichTreasuryChartData(
      getDataForPeriod("all", snapshot, "value", "eur", "detailed"),
      snapshot,
      "all",
    ).filter((point) => point.isActivityEvent);
    const byId = new Map(points.map((point) => [point.eventTransactionId, point]));

    expect(byId.get("before")?.avgCostEur).toBe(80_000);
    expect(byId.get("after")?.basisIncomplete).toBe(true);
    expect(byId.get("after")?.avgCostEur).toBeNull();
  });
});

describe("overview readiness completeness signals", () => {
  it("surfaces unresolved custody gaps instead of calling the book ready", () => {
    const snapshot = snapshotWith({
      ...QUARANTINE_FROM_JAN_3,
      reasons: ["custody_unresolved"],
      quarantineCount: 0,
    });

    expect(buildOverviewReadiness(snapshot).title.key).toBe(
      "readiness.custodyUnresolved.title",
    );
    expect(buildPrimaryOverviewAction(snapshot)?.href).toBe("/journals");
    const basis = buildOverviewHealthItems(snapshot).find(
      (item) => item.key === "basis",
    );
    expect(basis?.value.key).toBe("health.basis.custody");
  });

  it("surfaces missing prices with their count", () => {
    const snapshot = snapshotWith({
      ...QUARANTINE_FROM_JAN_3,
      reasons: ["missing_prices"],
      quarantineCount: 0,
      missingPriceCount: 3,
    });

    const readiness = buildOverviewReadiness(snapshot);
    expect(readiness.title.key).toBe("readiness.missingPrices.title");
    expect(readiness.detail.params).toEqual({ count: 3 });
    expect(
      buildOverviewHealthItems(snapshot).find((item) => item.key === "basis")
        ?.value,
    ).toEqual({ key: "health.basis.missingPrices", params: { count: 3 } });
  });

  it("adds no standing basis row for a complete book", () => {
    const snapshot = snapshotWith(MOCK_OVERVIEW.fiat.completeness!);

    expect(
      buildOverviewHealthItems(snapshot).some((item) => item.key === "basis"),
    ).toBe(false);
  });
});
