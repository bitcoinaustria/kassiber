import { Link } from "@tanstack/react-router";

import { Skeleton } from "@/components/ui/skeleton";
import { formatBtc, type Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import {
  activeMarketFiatCurrency,
  activeMarketFiatRate,
  blurClass,
  buildStatsData,
  currencyFormatter,
  formatCompactDisplayMoney,
  formatMarketRateValue,
  latestPortfolioBalanceBtc,
  marketRateCompactLabel,
  marketRateDetailLabel,
  numberFormatter,
} from "./model";

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
  const stats = buildStatsData(snapshot, currency);
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const fiatRate = activeMarketFiatRate(snapshot);
  const marketRateIsSynced = Boolean(
    snapshot.marketRate?.fetchedAt ?? snapshot.marketRate?.timestamp,
  );
  const marketRateDetail = marketRateDetailLabel(snapshot);
  return (
    <div
      className="rounded-xl border bg-card"
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
          disabled={!onRefreshMarketRate || isRefreshing || isMarketRateRefreshing}
          className="group relative isolate w-full overflow-hidden p-3 text-left transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/60 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-within:before:scale-x-100 disabled:cursor-default enabled:cursor-pointer sm:p-4"
          aria-label="Refresh BTC price"
        >
          {isRefreshing || isMarketRateRefreshing ? (
            <div className="pointer-events-none relative z-20 space-y-2">
              <Skeleton className="h-3 w-20" />
              <Skeleton className="h-6 w-28" />
              <div className="flex items-center gap-2">
                <Skeleton className="h-3 w-20" />
                <Skeleton className="hidden h-3 w-24 sm:block" />
              </div>
            </div>
          ) : (
            <div className="relative z-20 space-y-2">
              <div className="text-muted-foreground">
                <span className="text-xs font-medium">BTC price</span>
              </div>
              <p className="text-xl font-semibold tracking-tight">
                {formatMarketRateValue(snapshot)}
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
                {marketRateCompactLabel(snapshot)}
              </p>
            </div>
          )}
        </button>
        {stats.map((stat) => {
          const formatter =
            stat.format === "currency" ? currencyFormatter : numberFormatter;
          const hasComparison = stat.previousValue > 0;
          const statusText = hasComparison
            ? `${stat.isPositive ? "+" : "-"}${stat.changePercent.toFixed(1)}%`
            : stat.value === 0
              ? "Clear"
              : stat.title === "Portfolio value"
                ? "Estimate"
                : stat.title === "Transactions"
                  ? "Loaded"
                  : stat.title === "Connections"
                    ? "Configured"
                    : "Open";
          const isBitcoinPortfolio =
            currency === "btc" && stat.title === "Portfolio value";

          return (
            <div
              key={stat.title}
              className="group relative isolate overflow-hidden p-3 transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/60 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-within:before:scale-x-100 sm:p-4"
            >
              {isRefreshing ? (
                <div className="pointer-events-none relative z-20 space-y-2">
                  <Skeleton className="h-3 w-28" />
                  <Skeleton className="h-6 w-24" />
                  <div className="flex items-center gap-2">
                    <Skeleton className="h-3 w-16" />
                    <Skeleton className="hidden h-3 w-24 sm:block" />
                  </div>
                </div>
              ) : (
                <>
                  <Link
                    to={stat.href}
                    className="absolute inset-0 z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    aria-label={`Open ${isBitcoinPortfolio ? "Bitcoin balance" : stat.title}`}
                  />
                  <div className="pointer-events-none relative z-20 space-y-2">
                    <div className="text-muted-foreground">
                      <span className="text-xs font-medium">
                        {isBitcoinPortfolio ? "Bitcoin balance" : stat.title}
                      </span>
                    </div>
                    <p
                      className={cn(
                        "text-xl font-semibold tracking-tight",
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
                        formatter.format(stat.value)
                      )}
                    </p>
                    <div className="flex flex-wrap items-center gap-2 text-[10px] sm:text-xs xl:flex-nowrap">
                      <span
                        className={cn(
                          "font-medium",
                          stat.isPositive
                            ? "text-emerald-600 dark:text-emerald-400"
                            : "text-red-600 dark:text-red-400",
                          blurClass(hideSensitive),
                        )}
                      >
                        {statusText}
                        {hasComparison && (
                          <span className="hidden sm:inline">
                            (
                            {stat.format === "currency"
                              ? formatCompactDisplayMoney(
                                  Math.abs(stat.value - stat.previousValue),
                                  fiatRate,
                                  currency,
                                  fiatCurrency,
                                )
                              : formatter.format(
                                  Math.abs(stat.value - stat.previousValue),
                                )}
                            )
                          </span>
                        )}
                      </span>
                      <span className="hidden items-center gap-2 text-muted-foreground sm:inline-flex">
                        <span className="size-1 rounded-full bg-muted-foreground" />
                        <span className="xl:whitespace-nowrap">
                          {stat.comparisonLabel}
                        </span>
                      </span>
                    </div>
                  </div>
                </>
              )}
            </div>
          );
        })}
      </div>
      {isRefreshing ? (
        <span className="sr-only">Refreshing overview statistics</span>
      ) : null}
    </div>
  );
};
