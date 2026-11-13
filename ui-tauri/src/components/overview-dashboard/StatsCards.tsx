import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";

import { formatBtc } from "@/lib/currency";
import { fiatCompleteness } from "@/lib/fiatCompleteness";
import { maskSensitive, useSensitive } from "@/lib/sensitive";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import {
  activeMarketFiatCurrency,
  activeMarketFiatRate,
  blurClass,
  buildPortfolioStat,
  formatCompactDisplayMoney,
  formatMarketRateValue,
  displayBalanceBtc,
  marketRateCompactLabel,
  marketRateDetailLabel,
  portfolioCompletenessDetail,
  sourcesHoldingBitcoin,
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
  const hideShares = useSensitive("share");
  const to = t as OverviewTranslate;
  // The bitcoin held and what it is worth, both at once whichever currency
  // the app shows: the toggle would otherwise turn one into the other.
  const stat = buildPortfolioStat(snapshot);
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const fiatRate = activeMarketFiatRate(snapshot);
  const marketRateIsSynced = Boolean(
    snapshot.marketRate?.fetchedAt ?? snapshot.marketRate?.timestamp,
  );
  const marketRateDetail = marketRateDetailLabel(snapshot, to);
  const completeness = fiatCompleteness(snapshot.fiat);
  // Portfolio figures that are not exact: amber value, a badge in place of
  // the percentage, and what is missing instead of "vs cost basis".
  const completenessIssue = portfolioCompletenessDetail(snapshot);
  const balanceStatus = snapshot.balanceSummary ?? snapshot.status;
  const statusKey = statStatusKey(stat, balanceStatus, completeness);
  const statusText = statusKey
    ? t(statusKey, { count: balanceStatus?.quarantines ?? 0 })
    : maskSensitive(`${stat.isPositive ? "+" : "-"}${stat.changePercent.toFixed(1)}%`, hideShares);
  const portfolioWarningHref = completenessIssue
    ? completenessIssue.href
    : statusKey === "stats.status.needsJournals"
      ? "/journals"
      : statusKey === "stats.status.reviewQuarantines"
        ? "/review"
        : null;
  const portfolioWarning = Boolean(portfolioWarningHref);
  const valueUnavailable = Boolean(completenessIssue) && completeness.marketRateMissing;
  const statTitle = t("stats.title.portfolioValue");
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
        <div className="group relative isolate overflow-hidden p-3 transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/45 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-within:before:scale-x-100">
          {/* The balance is observed, not basis-derived, so it carries no
              status; it opens the wallets it is made of. */}
          <Link
            to="/connections"
            className="absolute inset-0 z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={t("stats.openStat", { title: t("stats.bitcoinBalance") })}
          />
          <div className="pointer-events-none relative z-20 space-y-1.5">
            <div className="text-muted-foreground">
              <span className="text-xs font-medium">{t("stats.bitcoinBalance")}</span>
            </div>
            <p
              className={cn(
                "text-lg font-semibold tracking-tight tabular-nums sm:text-xl",
                blurClass(hideSensitive),
              )}
            >
              {maskSensitive(
                formatBtc(displayBalanceBtc(snapshot), { precision: 3 }),
                hideSensitive,
              )}
            </p>
            <p className="truncate text-2xs text-muted-foreground sm:text-xs">
              {t("stats.heldIn", { count: sourcesHoldingBitcoin(snapshot) })}
            </p>
          </div>
        </div>
        <div className="group relative isolate overflow-hidden p-3 transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/45 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-within:before:scale-x-100">
          <Link
            to={portfolioWarningHref ?? stat.href}
            className="absolute inset-0 z-10 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={t("stats.openStat", { title: statTitle })}
            // The card body ignores pointer events, so the reason for an
            // amber/"—" value hangs on the link that receives the hover.
            title={completenessIssue ? t(completenessIssue.hintKey) : undefined}
          />
          <div className="pointer-events-none relative z-20 space-y-1.5">
            <div className="text-muted-foreground">
              <span className="text-xs font-medium">{statTitle}</span>
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
              ) : (
                <span>
                  {maskSensitive(
                    formatCompactDisplayMoney(stat.value, fiatRate, "eur", fiatCurrency),
                    hideSensitive,
                  )}
                </span>
              )}
            </p>
            <div className="flex min-w-0 items-center gap-1.5 text-2xs sm:text-xs">
              <span
                className={cn(
                  "shrink-0 font-medium",
                  portfolioWarning
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
                <span className={cn("min-w-0 truncate text-muted-foreground", blurClass(hideSensitive))}>
                  {/* dynamic key */}
                  {t(completenessIssue.copy.key as never, completenessIssue.copy.params)}
                </span>
              ) : (
                <span className="min-w-0 truncate text-muted-foreground">
                  {/* dynamic key */}
                  {t(stat.comparisonLabelKey as never)}
                </span>
              )}
            </div>
          </div>
        </div>
      </div>
      {isRefreshing ? (
        <span className="sr-only">{t("stats.refreshingStats")}</span>
      ) : null}
    </div>
  );
};
