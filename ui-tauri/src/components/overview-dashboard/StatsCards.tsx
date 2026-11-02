import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import { formatBtc } from "@/lib/currency";
import { fiatCompleteness } from "@/lib/fiatCompleteness";
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
  portfolioCompletenessDetail,
  type OverviewTranslate,
} from "./model";
import { statStatusKey } from "./statStatus";

export const StatsCards = ({
  snapshot,
  hideSensitive,
  isRefreshing,
  isMarketRateRefreshing,
  onRefreshMarketRate,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  isRefreshing?: boolean;
  isMarketRateRefreshing?: boolean;
  onRefreshMarketRate?: () => void;
}) => {
  const { t } = useTranslation("overview");
  const to = t as OverviewTranslate;
  // The bitcoin held and what it is worth, both at once whichever currency
  // the app shows: the toggle would otherwise turn one into the other.
  const tiles = [
    { stat: buildStatsData(snapshot, "btc")[0], isBitcoinPortfolio: true },
    { stat: buildStatsData(snapshot, "eur")[0], isBitcoinPortfolio: false },
  ];
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const fiatRate = activeMarketFiatRate(snapshot);
  const marketRateIsSynced = Boolean(
    snapshot.marketRate?.fetchedAt ?? snapshot.marketRate?.timestamp,
  );
  const marketRateDetail = marketRateDetailLabel(snapshot, to);
  const completeness = fiatCompleteness(snapshot.fiat);
  const completenessDetail = portfolioCompletenessDetail(snapshot);
  return (
    <div
      className="kb-surface overflow-hidden"
      role={isRefreshing ? "status" : undefined}
      aria-live={isRefreshing ? "polite" : undefined}
    >
      <div className="grid grid-cols-1 divide-y divide-border sm:grid-cols-3 sm:divide-x sm:divide-y-0">
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
                <span className="text-2xs font-medium text-primary">
                  {t("stats.refreshing")}
                </span>
              ) : null}
            </div>
            {/* `tabular-nums`: these tiles sit side by side, so proportional
                digits leave the figures misaligned across tiles and make them
                jitter as values update mid-sync. */}
            <p className="text-lg font-semibold tracking-tight tabular-nums sm:text-xl">
              {formatMarketRateValue(snapshot, to)}
            </p>
            <p
              className={cn(
                "truncate text-2xs font-medium leading-tight sm:text-xs",
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
        {tiles.map(({ stat, isBitcoinPortfolio }) => {
          const balanceStatus = snapshot.balanceSummary ?? snapshot.status;
          const statusKey = statStatusKey(
            stat,
            isBitcoinPortfolio,
            balanceStatus,
            completeness,
          );
          // Fiat portfolio figures that are not exact: amber value, a badge in
          // place of the percentage, and what is missing instead of "vs cost
          // basis". The BTC balance is observed and keeps its own statuses.
          const completenessIssue =
            stat.id === "portfolioValue" && !isBitcoinPortfolio
              ? completenessDetail
              : null;
          const statusText = statusKey
            ? t(statusKey, {
                count: balanceStatus?.quarantines ?? 0,
              })
            : `${stat.isPositive ? "+" : "-"}${stat.changePercent.toFixed(1)}%`;
          const balanceWarning =
            Boolean(completenessIssue) ||
            statusKey === "stats.status.needsJournals" ||
            statusKey === "stats.status.reviewQuarantines";
          const balanceWarningHref = completenessIssue
            ? completenessIssue.href
            : statusKey === "stats.status.needsJournals"
              ? "/journals"
              : statusKey === "stats.status.reviewQuarantines"
                ? "/review"
                : null;
          const showComparisonLabel =
            !completenessIssue &&
            (!statusKey ||
              ![
                "stats.status.current",
              ].includes(statusKey));
          const valueUnavailable =
            Boolean(completenessIssue) && completeness.marketRateMissing;
          const statTitle = isBitcoinPortfolio
            ? t("stats.bitcoinBalance")
            : // dynamic key
              t(stat.titleKey as never);

          return (
            <div
              key={isBitcoinPortfolio ? "bitcoinBalance" : stat.id}
              className="group relative isolate overflow-hidden p-3 transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/45 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-within:before:scale-x-100"
            >
              <>
                <Link
                  // The Bitcoin balance belongs to the wallets overview — that
                  // is where the balance is made of parts. (That page is routed
                  // at `/connections`, which is also where the nav's "Wallets"
                  // row goes.) The portfolio value stays on Reports.
                  to={
                    balanceWarningHref ??
                    (isBitcoinPortfolio ? "/connections" : stat.href)
                  }
                  className="absolute inset-0 z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  aria-label={t("stats.openStat", { title: statTitle })}
                  // The card body ignores pointer events, so the reason for an
                  // amber/"—" value hangs on the link that receives the hover.
                  title={
                    completenessIssue ? t(completenessIssue.hintKey) : undefined
                  }
                />
                <div className="pointer-events-none relative z-20 space-y-1.5">
                  <div className="text-muted-foreground">
                    <span className="text-xs font-medium">
                      {statTitle}
                    </span>
                  </div>
                  <p
                    className={cn(
                      "text-lg font-semibold tracking-tight tabular-nums sm:text-xl",
                      completenessIssue && "text-amber-600 dark:text-amber-400",
                      blurClass(hideSensitive),
                    )}
                  >
                    {valueUnavailable ? (
                      <span>—</span>
                    ) : isBitcoinPortfolio ? (
                      <span>
                        {formatBtc(latestPortfolioBalanceBtc(snapshot), {
                          precision: 3,
                        })}
                      </span>
                    ) : (
                      <span>
                        {formatCompactDisplayMoney(
                          stat.value,
                          fiatRate,
                          "eur",
                          fiatCurrency,
                        )}
                      </span>
                    )}
                  </p>
                  <div className="flex min-w-0 items-center gap-1.5 text-2xs sm:text-xs">
                    <span
                      className={cn(
                        "shrink-0 font-medium",
                        balanceWarning
                          ? "text-amber-600 dark:text-amber-400"
                          : stat.isPositive
                          ? "text-emerald-600 dark:text-emerald-400"
                          : "text-red-600 dark:text-red-400",
                        blurClass(hideSensitive),
                      )}
                    >
                      {statusText}
                    </span>
                    {completenessIssue ? (
                      <span
                        className={cn(
                          "min-w-0 truncate text-muted-foreground",
                          blurClass(hideSensitive),
                        )}
                      >
                        {/* dynamic key */}
                        {t(
                          completenessIssue.copy.key as never,
                          completenessIssue.copy.params,
                        )}
                      </span>
                    ) : showComparisonLabel ? (
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
