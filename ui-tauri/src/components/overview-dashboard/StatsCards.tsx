import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import {
  fiatNumberFormatter,
  formatBtc,
  type Currency,
} from "@/lib/currency";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import {
  activeMarketFiatCurrency,
  activeMarketFiatRate,
  blurClass,
  buildStatsData,
  formatCompactDisplayMoney,
  formatMarketRateValue,
  latestPortfolioBalanceBtc,
  marketRateCompactLabel,
  marketRateDetailLabel,
  type OverviewTranslate,
  type StatItem,
} from "./model";

const numberFormatter = fiatNumberFormatter("EUR");

export function statStatusKey(stat: StatItem, isBitcoinPortfolio: boolean) {
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

export const StatsCards = ({
  snapshot,
  hideSensitive,
  currency,
  isRefreshing,
  isMarketRateRefreshing,
  onRefreshMarketRate,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
  isRefreshing?: boolean;
  isMarketRateRefreshing?: boolean;
  onRefreshMarketRate?: () => void;
}) => {
  const { t } = useTranslation("overview");
  const to = t as OverviewTranslate;
  const stats = buildStatsData(snapshot, currency);
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const fiatRate = activeMarketFiatRate(snapshot);
  const marketRateIsSynced = Boolean(
    snapshot.marketRate?.fetchedAt ?? snapshot.marketRate?.timestamp,
  );
  const marketRateDetail = marketRateDetailLabel(snapshot, to);
  return (
    <div
      className="overflow-hidden rounded-lg border bg-card"
      role={isRefreshing ? "status" : undefined}
      aria-live={isRefreshing ? "polite" : undefined}
    >
      <div className="grid grid-cols-1 divide-x-0 divide-y divide-border sm:grid-cols-2 sm:divide-y-0 xl:grid-cols-5 xl:divide-x">
        <button
          type="button"
          onClick={(event) => {
            event.preventDefault();
            onRefreshMarketRate?.();
          }}
          disabled={
            !onRefreshMarketRate || isRefreshing || isMarketRateRefreshing
          }
          className="group relative isolate w-full overflow-hidden p-3 text-left transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/45 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-within:before:scale-x-100 disabled:cursor-default enabled:cursor-pointer"
          aria-label={t("stats.refreshBtcPrice")}
        >
          <div className="relative z-20 space-y-1.5">
            <div className="flex items-center justify-between gap-2 text-muted-foreground">
              <span className="text-xs font-medium">{t("stats.btcPrice")}</span>
              {isMarketRateRefreshing ? (
                <span className="text-[10px] font-medium text-primary">
                  {t("stats.refreshing")}
                </span>
              ) : null}
            </div>
            <p className="text-lg font-semibold tracking-tight sm:text-xl">
              {formatMarketRateValue(snapshot, to)}
            </p>
            <p
              className={cn(
                "truncate text-[10px] font-medium leading-tight sm:text-xs",
                marketRateIsSynced
                  ? "text-emerald-600 dark:text-emerald-400"
                  : "text-muted-foreground",
              )}
              title={marketRateDetail}
            >
              {marketRateCompactLabel(snapshot, to)}
            </p>
          </div>
        </button>
        {stats.map((stat) => {
          const isBitcoinPortfolio =
            currency === "btc" && stat.id === "portfolioValue";
          const statusKey = statStatusKey(stat, isBitcoinPortfolio);
          const statusText = statusKey
            ? t(statusKey)
            : `${stat.isPositive ? "+" : "-"}${stat.changePercent.toFixed(1)}%`;
          const showComparisonLabel =
            !statusKey ||
            ![
              "stats.status.current",
              "stats.status.loaded",
              "stats.status.configured",
            ].includes(statusKey);
          const statTitle = isBitcoinPortfolio
            ? t("stats.bitcoinBalance")
            : // dynamic key
              t(stat.titleKey as never);

          return (
            <div
              key={stat.id}
              className="group relative isolate overflow-hidden p-3 transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/45 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-within:before:scale-x-100"
            >
              <>
                <Link
                  to={stat.href}
                  className="absolute inset-0 z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  aria-label={t("stats.openStat", { title: statTitle })}
                />
                <div className="pointer-events-none relative z-20 space-y-1.5">
                  <div className="text-muted-foreground">
                    <span className="text-xs font-medium">
                      {statTitle}
                    </span>
                  </div>
                  <p
                    className={cn(
                      "text-lg font-semibold tracking-tight sm:text-xl",
                      blurClass(hideSensitive),
                    )}
                  >
                    {isBitcoinPortfolio ? (
                      <span>
                        {formatBtc(latestPortfolioBalanceBtc(snapshot), {
                          precision: 3,
                        })}
                      </span>
                    ) : stat.format === "currency" ? (
                      <span>
                        {formatCompactDisplayMoney(
                          stat.value,
                          fiatRate,
                          currency,
                          fiatCurrency,
                        )}
                      </span>
                    ) : (
                      numberFormatter.format(stat.value)
                    )}
                  </p>
                  <div className="flex min-w-0 items-center gap-1.5 text-[10px] sm:text-xs">
                    <span
                      className={cn(
                        "shrink-0 font-medium",
                        stat.isPositive
                          ? "text-emerald-600 dark:text-emerald-400"
                          : "text-red-600 dark:text-red-400",
                        blurClass(hideSensitive),
                      )}
                    >
                      {statusText}
                    </span>
                    {showComparisonLabel ? (
                      <span className="min-w-0 truncate text-muted-foreground">
                        {/* dynamic key */}
                        {t(stat.comparisonLabelKey as never)}
                      </span>
                    ) : null}
                  </div>
                </div>
              </>
            </div>
          );
        })}
      </div>
      {isRefreshing ? (
        <span className="sr-only">{t("stats.refreshingStats")}</span>
      ) : null}
    </div>
  );
};
