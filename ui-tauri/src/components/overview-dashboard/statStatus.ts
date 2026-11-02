import type { FiatCompleteness, OverviewSnapshot } from "@/mocks/seed";

import type { StatItem } from "./model";

export type BalanceStatus = Pick<
  NonNullable<OverviewSnapshot["balanceSummary"]>,
  "needsJournals" | "quarantines"
>;

export type PortfolioCompletenessKey =
  | "completeness.badge.noMarketRate"
  | "completeness.badge.stale"
  | "completeness.badge.incomplete"
  | "completeness.badge.unavailable";

/**
 * Badge for the fiat portfolio card when its figures are not exact. The BTC
 * balance is observed, not basis-derived, so its tile carries no status.
 */
export function portfolioCompletenessKey(
  completeness: FiatCompleteness | undefined,
): PortfolioCompletenessKey | null {
  if (!completeness) return null;
  if (completeness.marketRateMissing) return "completeness.badge.noMarketRate";
  if (completeness.costBasisComplete) return null;
  if (completeness.state === "stale") return "completeness.badge.stale";
  if (completeness.state === "unavailable") return "completeness.badge.unavailable";
  return "completeness.badge.incomplete";
}

export function statStatusKey(
  stat: StatItem,
  balanceStatus?: BalanceStatus,
  completeness?: FiatCompleteness,
) {
  const completenessKey = portfolioCompletenessKey(completeness);
  if (completenessKey) return completenessKey;
  if (balanceStatus?.needsJournals) return "stats.status.needsJournals";
  if ((balanceStatus?.quarantines ?? 0) > 0) {
    return "stats.status.reviewQuarantines";
  }
  if (stat.previousValue > 0) return null;
  if (stat.value === 0) return "stats.status.clear";
  return "stats.status.estimate";
}

// English status text, kept for non-UI callers (tests). UI components resolve
// `statStatusKey()` through i18next instead.
const STAT_STATUS_EN: Record<string, string> = {
  "stats.status.clear": "Clear",
  "stats.status.estimate": "Estimate",
  "completeness.badge.noMarketRate": "No market rate",
  "completeness.badge.stale": "Outdated",
  "completeness.badge.incomplete": "Incomplete",
  "completeness.badge.unavailable": "Unavailable",
};

export function statStatusText(stat: StatItem, completeness?: FiatCompleteness) {
  const key = statStatusKey(stat, undefined, completeness);
  if (!key) {
    return `${stat.isPositive ? "+" : "-"}${stat.changePercent.toFixed(1)}%`;
  }
  return STAT_STATUS_EN[key] ?? key;
}
