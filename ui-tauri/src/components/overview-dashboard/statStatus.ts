import type { OverviewSnapshot } from "@/mocks/seed";

import type { StatItem } from "./model";

export type BalanceStatus = Pick<
  NonNullable<OverviewSnapshot["balanceSummary"]>,
  "needsJournals" | "quarantines"
>;

export function statStatusKey(
  stat: StatItem,
  isBitcoinPortfolio: boolean,
  balanceStatus?: BalanceStatus,
) {
  if (stat.id === "portfolioValue" && balanceStatus?.needsJournals) {
    return "stats.status.needsJournals";
  }
  if (stat.id === "portfolioValue" && (balanceStatus?.quarantines ?? 0) > 0) {
    return "stats.status.reviewQuarantines";
  }
  if (stat.previousValue > 0) {
    return null;
  }
  if (isBitcoinPortfolio) return "stats.status.current";
  if (stat.value === 0) return "stats.status.clear";
  if (stat.id === "portfolioValue") return "stats.status.estimate";
  if (stat.id === "transactions") return "stats.status.loaded";
  if (stat.id === "connections") return "stats.status.configured";
  return "stats.status.open";
}

// English status text, kept for non-UI callers (tests). UI components resolve
// `statStatusKey()` through i18next instead.
const STAT_STATUS_EN: Record<string, string> = {
  "stats.status.current": "Current",
  "stats.status.clear": "Clear",
  "stats.status.estimate": "Estimate",
  "stats.status.loaded": "Loaded",
  "stats.status.configured": "Configured",
  "stats.status.open": "Open",
};

export function statStatusText(stat: StatItem, isBitcoinPortfolio: boolean) {
  const key = statStatusKey(stat, isBitcoinPortfolio);
  if (!key) {
    return `${stat.isPositive ? "+" : "-"}${stat.changePercent.toFixed(1)}%`;
  }
  return STAT_STATUS_EN[key] ?? key;
}
