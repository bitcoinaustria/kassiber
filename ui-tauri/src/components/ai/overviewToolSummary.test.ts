import { describe, expect, it } from "vitest";

import i18n from "@/i18n";
import { MOCK_OVERVIEW } from "@/mocks/seed";

import { overviewSnapshotSummary } from "./overviewToolSummary";

const t = () => i18n.getFixedT("en", "assistant");

function overviewResult(fiat: Record<string, unknown>) {
  return { connections: [], txs: [], priceEur: 60_000, fiat };
}

describe("overview tool summary", () => {
  it("states realized YTD plainly when the basis is complete", () => {
    const summary = overviewSnapshotSummary(
      overviewResult({ ...MOCK_OVERVIEW.fiat, eurRealizedYTD: 1_000 }),
      t(),
    );

    expect(summary).toContain("realized YTD");
    expect(summary).not.toContain("incomplete");
  });

  it("qualifies realized YTD while the basis is incomplete", () => {
    const summary = overviewSnapshotSummary(
      overviewResult({
        ...MOCK_OVERVIEW.fiat,
        eurRealizedYTD: 1_000,
        completeness: {
          ...MOCK_OVERVIEW.fiat.completeness,
          state: "incomplete",
          costBasisComplete: false,
          reasons: ["quarantines"],
          quarantineCount: 1,
        },
      }),
      t(),
    );

    expect(summary).toContain("(incomplete)");
    expect(summary).toContain("cost basis incomplete");
  });

  it("does not present a fallback price as the market rate", () => {
    const summary = overviewSnapshotSummary(
      overviewResult({
        ...MOCK_OVERVIEW.fiat,
        completeness: {
          ...MOCK_OVERVIEW.fiat.completeness,
          state: "unavailable",
          reasons: ["market_rate_missing"],
          marketRateMissing: true,
        },
      }),
      t(),
    );

    expect(summary).toContain("no market rate");
    expect(summary).not.toContain("BTC/EUR");
  });
});
