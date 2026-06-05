import { describe, expect, it } from "vitest";

import { MOCK_OVERVIEW } from "@/mocks/seed";

import { buildStatsData } from "./model";
import { statStatusText } from "./StatsCards";

describe("overview stats cards", () => {
  it("does not label the BTC balance as an estimate", () => {
    const bitcoinBalanceStat = buildStatsData(MOCK_OVERVIEW, "btc")[0];
    const zeroBitcoinBalanceStat = { ...bitcoinBalanceStat, value: 0 };
    const fiatPortfolioStat = {
      ...buildStatsData(MOCK_OVERVIEW, "eur")[0],
      previousValue: 0,
    };

    expect(statStatusText(bitcoinBalanceStat, true)).toBe("Current");
    expect(statStatusText(zeroBitcoinBalanceStat, true)).toBe("Current");
    expect(statStatusText(fiatPortfolioStat, false)).toBe("Estimate");
  });
});
