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
 * balance is observed, not basis-derived, so it keeps the journal/quarantine
 * statuses below instead.
 */
export function portfolioCompletenessKey(
  completeness: FiatCompleteness | undefined,
  isBitcoinPortfolio: boolean,
): PortfolioCompletenessKey | null {
  if (isBitcoinPortfolio || !completeness) return null;
  if (completeness.marketRateMissing) return "completeness.badge.noMarketRate";
  if (completeness.costBasisComplete) return null;
  if (completeness.state === "stale") return "completeness.badge.stale";
  if (completeness.state === "unavailable") return "completeness.badge.unavailable";
  return "completeness.badge.incomplete";
}

export function statStatusKey(
  stat: StatItem,
  isBitcoinPortfolio: boolean,
  balanceStatus?: BalanceStatus,
  completeness?: FiatCompleteness,
) {
  if (stat.id === "portfolioValue") {
    const completenessKey = portfolioCompletenessKey(
      completeness,
      isBitcoinPortfolio,
    );
    if (completenessKey) return completenessKey;
  }
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
  "completeness.badge.noMarketRate": "No market rate",
  "completeness.badge.stale": "Outdated",
  "completeness.badge.incomplete": "Incomplete",
  "completeness.badge.unavailable": "Unavailable",
};

export function statStatusText(
  stat: StatItem,
  isBitcoinPortfolio: boolean,
  completeness?: FiatCompleteness,
) {
  const key = statStatusKey(stat, isBitcoinPortfolio, undefined, completeness);
  if (!key) {
    return `${stat.isPositive ? "+" : "-"}${stat.changePercent.toFixed(1)}%`;
  }
  return STAT_STATUS_EN[key] ?? key;
}
