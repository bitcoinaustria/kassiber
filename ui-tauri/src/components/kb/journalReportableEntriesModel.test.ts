import { describe, expect, it } from "vitest";

import {
  reportableEntryMetricFilterIds,
  reportableEntryMetrics,
} from "@/components/kb/journalReportableEntriesModel";

describe("Reportable journal entry quick filters", () => {
  it("tags reportable entries for disjoint metric-card filtering", () => {
    expect(
      reportableEntryMetricFilterIds({
        entryType: "disposal",
        gainLossEur: -12.5,
      }),
    ).toEqual(["disposals"]);
    expect(
      reportableEntryMetricFilterIds({ entryType: "fee", gainLossEur: null }),
    ).toEqual(["fees"]);
    expect(
      reportableEntryMetricFilterIds({
        entryType: "neutral_swap",
        gainLossEur: 0,
      }),
    ).toEqual(["neutral"]);
    expect(
      reportableEntryMetricFilterIds({
        entryType: "acquisition",
        gainLossEur: 0,
      }),
    ).toEqual(["acquisitions"]);
    expect(
      reportableEntryMetricFilterIds({
        entryType: "income",
        gainLossEur: null,
      }),
    ).toEqual(["income"]);
  });

  it("exposes metric filter ids for the summary card row", () => {
    const metrics = reportableEntryMetrics({
      workspace: "book",
      profile: "at",
      count: 113,
      reportableCount: 3,
      needsJournals: false,
      lastProcessedAt: "2026-05-13T20:00:00Z",
      freshnessStatus: "current",
      freshnessReason: "journals match the active transaction count",
      limit: 100,
      entryTypes: [
        { type: "acquisition", count: 109, gainLossEur: 0 },
        { type: "disposal", count: 2, gainLossEur: -681.41 },
        { type: "fee", count: 1, gainLossEur: 0 },
        { type: "neutral_swap", count: 1, gainLossEur: 0 },
      ],
    });

    expect(metrics.map((metric) => metric.filterId)).toEqual([
      "acquisitions",
      "disposals",
      "income",
      "fees",
      "neutral",
    ]);
    expect(metrics.map((metric) => metric.value)).toEqual([
      109,
      2,
      0,
      1,
      1,
    ]);
  });
});
