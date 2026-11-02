import { Link } from "@tanstack/react-router";
import { ArrowUpRight } from "lucide-react";
import { useTranslation } from "react-i18next";

import { RailIcon } from "@/components/kb/BitcoinRail";
import { type Currency } from "@/lib/currency";
import { fiatCompleteness } from "@/lib/fiatCompleteness";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import {
  activeMarketFiatCurrency,
  activeMarketFiatRate,
  blurClass,
  buildHoldingsBySource,
  formatCompactDisplayMoney,
  portfolioCompletenessDetail,
  useHoverHighlight,
} from "./model";

/** Sources listed before the rest fold into "Other sources". */
const SHOWN_SOURCES = 8;

/**
 * Where the bitcoin sits, across the page under the chart: one bar split by
 * source, and each source with its rail's logo, value and share. Hovering a
 * source lights up its part of the bar.
 */
export const HoldingsBySourceChart = ({
  snapshot,
  hideSensitive,
  currency,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
}) => {
  const { t } = useTranslation("overview");
  const isBitcoinMode = currency === "btc";
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const fiatRate = activeMarketFiatRate(snapshot);
  const { active, handleHover } = useHoverHighlight<number>();
  const holdings = buildHoldingsBySource(snapshot, SHOWN_SOURCES);
  const completenessDetail = portfolioCompletenessDetail(snapshot);
  const unrealizedPercent = snapshot.fiat.eurCostBasis
    ? (snapshot.fiat.eurUnrealized / snapshot.fiat.eurCostBasis) * 100
    : 0;
  const total = holdings.reduce((sum, item) => sum + item.value, 0);
  // Without a market rate a fiat figure would be a stale or zero placeholder.
  const fiatValueUnavailable =
    !isBitcoinMode && fiatCompleteness(snapshot.fiat).marketRateMissing;
  const valueLabel = (value: number) =>
    fiatValueUnavailable
      ? "—"
      : formatCompactDisplayMoney(value, fiatRate, currency, fiatCurrency);

  return (
    <section className="kb-surface space-y-3 p-3 sm:px-4" aria-label={t("holdings.aria")}>
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-0.5">
          <h2 className="text-sm font-medium">{t("holdings.title")}</h2>
          <span className={cn("text-sm font-semibold tabular-nums", blurClass(hideSensitive))}>
            {valueLabel(total)}
          </span>
          {isBitcoinMode ? null : completenessDetail ? (
            // No "+X % vs cost basis" while the basis misses rows or the
            // value has no market rate; say what is missing instead.
            <p
              className={cn(
                "truncate text-xs text-amber-600 dark:text-amber-400",
                // The copy can carry the uncovered BTC amount.
                completenessDetail.copy.params && blurClass(hideSensitive),
              )}
              title={t(completenessDetail.hintKey)}
            >
              {/* dynamic key */}
              {t(completenessDetail.copy.key as never, completenessDetail.copy.params)}
            </p>
          ) : (
            <p className="text-xs text-muted-foreground">
              <span
                className={cn(
                  "font-medium",
                  unrealizedPercent >= 0 ? "text-emerald-600 dark:text-emerald-400" : "text-red-600 dark:text-red-400",
                  blurClass(hideSensitive),
                )}
              >
                {unrealizedPercent >= 0 ? "+" : ""}
                {unrealizedPercent.toFixed(1)}%
              </span>{" "}
              {t("holdings.vsCostBasis")}
            </p>
          )}
        </div>
        <Link
          to="/connections"
          className="inline-flex shrink-0 items-center gap-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          {t("holdings.viewConnections")}
          <ArrowUpRight className="size-3" aria-hidden="true" />
        </Link>
      </div>

      {holdings.length ? (
        <>
          <div className="flex h-2 gap-0.5 overflow-hidden rounded-full" aria-hidden="true">
            {holdings.map((item, index) => (
              <span
                key={item.nameKey ?? item.name}
                className={cn(
                  "h-full transition-opacity duration-200 motion-reduce:transition-none",
                  active !== null && active !== index && "opacity-30",
                )}
                style={{ width: `${total ? (item.value / total) * 100 : 0}%`, backgroundColor: item.color }}
              />
            ))}
          </div>
          <ul className="grid gap-x-6 gap-y-0.5 sm:grid-cols-2 lg:grid-cols-3 2xl:grid-cols-4">
            {holdings.map((item, index) => (
              <li
                key={item.nameKey ?? item.name}
                className={cn(
                  "flex min-w-0 items-center gap-2 py-1 text-xs transition-opacity duration-200 motion-reduce:transition-none",
                  active !== null && active !== index && "opacity-50",
                )}
                onMouseEnter={() => handleHover(index)}
                onMouseLeave={() => handleHover(null)}
              >
                <span className="size-2 shrink-0 rounded-full" style={{ backgroundColor: item.color }} aria-hidden="true" />
                {item.rail ? <RailIcon rail={item.rail} size="compact" /> : null}
                <span className="min-w-0 flex-1 truncate" title={item.nameKey ? undefined : item.name}>
                  {/* dynamic key */}
                  {item.nameKey ? t(item.nameKey as never) : item.name}
                </span>
                <span className={cn("shrink-0 font-medium tabular-nums", blurClass(hideSensitive))}>
                  {valueLabel(item.value)}
                </span>
                <span className={cn("w-9 shrink-0 text-right text-muted-foreground tabular-nums", blurClass(hideSensitive))}>
                  {item.percent}%
                </span>
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </section>
  );
};
