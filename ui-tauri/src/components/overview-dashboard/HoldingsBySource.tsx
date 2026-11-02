import { Link } from "@tanstack/react-router";
import type { CSSProperties } from "react";
import { ArrowUpRight } from "lucide-react";
import { useTranslation } from "react-i18next";

import { RailIcon } from "@/components/kb/BitcoinRail";
import { formatBtc, formatWholeFiatAmount, type Currency } from "@/lib/currency";
import { fiatCompleteness } from "@/lib/fiatCompleteness";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import {
  activeMarketFiatCurrency,
  activeMarketFiatRate,
  blurClass,
  btcFromEur,
  buildHoldingsBySource,
  portfolioCompletenessDetail,
} from "./model";

/** Sources listed before the rest fold into "Other sources". */
const SHOWN_SOURCES = 8;

/**
 * Where the bitcoin sits, across the page under the chart: the sources
 * largest first, each with its rail's logo, value and share.
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
      : isBitcoinMode
        ? formatBtc(btcFromEur(value, fiatRate), { precision: 3 })
        : formatWholeFiatAmount(value, fiatCurrency);

  return (
    <section className="@container/holdings kb-surface space-y-2.5 p-3 sm:px-4" aria-label={t("holdings.aria")}>
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
        // Largest first, read down the first column and then the next; the
        // fixed value column keeps the figures of both columns in line.
        <ul
          className={cn(
            "grid gap-x-8 gap-y-1",
            holdings.length > 1 &&
              "@min-[44rem]/holdings:grid-flow-col @min-[44rem]/holdings:grid-cols-2 @min-[44rem]/holdings:grid-rows-[repeat(var(--holding-rows),auto)]",
          )}
          style={{ "--holding-rows": Math.ceil(holdings.length / 2) } as CSSProperties}
        >
          {holdings.map((item) => (
            <li
              key={item.nameKey ?? item.name}
              className="grid min-w-0 grid-cols-[1.5rem_minmax(0,1fr)_6.5rem_2.5rem] items-center gap-2 py-0.5 text-xs"
            >
              <span className="flex justify-center">
                {item.rail ? <RailIcon rail={item.rail} size="compact" /> : null}
              </span>
              <span
                className={cn("truncate", item.nameKey && "text-muted-foreground")}
                title={item.nameKey ? undefined : item.name}
              >
                {/* dynamic key */}
                {item.nameKey ? t(item.nameKey as never) : item.name}
              </span>
              <span className={cn("truncate text-right font-medium tabular-nums", blurClass(hideSensitive))}>
                {valueLabel(item.value)}
              </span>
              <span className={cn("text-right text-muted-foreground tabular-nums", blurClass(hideSensitive))}>
                {item.percent === 0 && item.value > 0 ? "<1%" : `${item.percent}%`}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
};
