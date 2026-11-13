import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";

import { formatBtc, formatFiatAmount, formatWholeFiatAmount, type Currency } from "@/lib/currency";
import { fiatCompleteness } from "@/lib/fiatCompleteness";
import { maskSensitive, useSensitive } from "@/lib/sensitive";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import { activeMarketFiatCurrency, blurClass, monthlyFlows, type MonthFlow } from "./model";

const WidgetCard = ({
  title,
  aside,
  children,
  className,
}: {
  title: ReactNode;
  aside?: ReactNode;
  children: ReactNode;
  className?: string;
}) => (
  <section className={cn("kb-surface flex min-w-0 flex-col gap-3 p-3 sm:px-4", className)}>
    <div className="flex items-baseline justify-between gap-3">
      <h2 className="text-sm font-medium">{title}</h2>
      {aside}
    </div>
    {children}
  </section>
);

const linkClass =
  "inline-flex items-center gap-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

/**
 * This year in tax terms: realized gains so far, what is unrealized against
 * cost basis, and in an Austrian book what is tax-free old stock. Every
 * figure is the daemon's; with an incomplete basis it says so instead.
 */
export const TaxYearWidget = ({
  snapshot,
  hideSensitive,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
}) => {
  const { t } = useTranslation("overview");
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const completeness = fiatCompleteness(snapshot.fiat);
  const basisKnown = completeness.costBasisComplete;
  // The daemon counts "this year" from 1 January UTC; the label names the
  // same year, or the turn of the year would put one year's figure under the next.
  const year = new Date().getUTCFullYear();
  const money = (value: number) => formatFiatAmount(value, fiatCurrency);
  const signed = (value: number) => `${value >= 0 ? "+ " : "− "}${money(Math.abs(value))}`;
  const tone = (value: number) =>
    value >= 0 ? "text-emerald-600 dark:text-emerald-400" : "text-red-600 dark:text-red-400";
  const taxFree = snapshot.taxFreeBalance;
  const rows: Array<{ label: string; value: string; className?: string }> = [
    {
      label: t("widgets.tax.realized"),
      value: basisKnown ? signed(snapshot.fiat.eurRealizedYTD) : "—",
      className: basisKnown ? tone(snapshot.fiat.eurRealizedYTD) : undefined,
    },
    {
      label: t("widgets.tax.unrealized"),
      value: basisKnown && !completeness.marketRateMissing ? signed(snapshot.fiat.eurUnrealized) : "—",
      className: basisKnown ? tone(snapshot.fiat.eurUnrealized) : undefined,
    },
    {
      label: t("widgets.tax.costBasis"),
      value: basisKnown ? money(snapshot.fiat.eurCostBasis) : "—",
    },
  ];
  if (taxFree) {
    rows.push(
      {
        label: t("widgets.tax.taxFree"),
        value: formatBtc(taxFree.taxFreeQuantitySats / 100_000_000, { precision: 8 }),
      },
      {
        label: t("widgets.tax.taxable"),
        value: formatBtc(taxFree.taxableQuantitySats / 100_000_000, { precision: 8 }),
      },
    );
  }
  return (
    <WidgetCard
      title={t("widgets.tax.title", { year })}
      aside={
        <Link to="/reports" className={linkClass}>
          {t("widgets.tax.openReports")}
          <ChevronRight className="size-3" aria-hidden="true" />
        </Link>
      }
    >
      <dl className="space-y-1.5 text-xs">
        {rows.map((row) => (
          <div key={row.label} className="flex items-baseline justify-between gap-3">
            <dt className="text-muted-foreground">{row.label}</dt>
            <dd className={cn("font-medium tabular-nums", row.className, blurClass(hideSensitive))}>
              {row.value === "—" ? row.value : maskSensitive(row.value, hideSensitive)}
            </dd>
          </div>
        ))}
      </dl>
      {basisKnown ? null : (
        <p className="text-2xs text-amber-700 dark:text-amber-300">{t("widgets.tax.basisIncomplete")}</p>
      )}
    </WidgetCard>
  );
};

/**
 * Money in and out per month over the last year: received above the line,
 * spent below, totals and fees underneath. A year without any is left out
 * rather than shown as a row of zeros.
 */
export const FlowsWidget = ({
  snapshot,
  hideSensitive,
  currency,
}: {
  snapshot: OverviewSnapshot;
  hideSensitive: boolean;
  currency: Currency;
}) => {
  const { t, i18n } = useTranslation("overview");
  // "Hide all": the bars' heights tell how money moved, so they go too.
  const hideShares = useSensitive("share");
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const flows = monthlyFlows(snapshot);
  if (!flows.hasActivity) return null;
  const { months } = flows;
  const isBitcoinMode = currency === "btc";
  const peak = Math.max(...months.map((month) => Math.max(month.inBtc, month.outBtc)), 0) || 1;
  const amount = (btc: number, value: number) =>
    maskSensitive(
      isBitcoinMode ? formatBtc(btc, { precision: 4 }) : formatWholeFiatAmount(value, fiatCurrency),
      hideSensitive,
    );
  const monthLabel = (date: Date) => date.toLocaleDateString(i18n.language, { month: "narrow" });
  const monthTitle = (month: MonthFlow) => {
    const name = month.date.toLocaleDateString(i18n.language, { month: "long", year: "numeric" });
    return hideSensitive
      ? name
      : `${name}: ${t("widgets.flows.received")} ${formatBtc(month.inBtc, { precision: 4 })} · ${t("widgets.flows.spent")} ${formatBtc(month.outBtc, { precision: 4 })}`;
  };
  return (
    <WidgetCard title={t("widgets.flows.title")}>
      <div
        className={cn("grid h-28 grid-cols-12 items-stretch gap-1", blurClass(hideSensitive))}
        role="img"
        aria-label={t("widgets.flows.chartAria")}
      >
        {months.map((month) => (
          <div key={month.key} className="flex min-w-0 flex-col" title={monthTitle(month)}>
            <div className="flex flex-1 items-end">
              {hideShares ? null : (
                <div
                  className="w-full rounded-t-sm bg-emerald-500/70"
                  style={{ height: `${(month.inBtc / peak) * 100}%`, minHeight: month.inBtc > 0 ? 2 : 0 }}
                />
              )}
            </div>
            <div className="h-px bg-border" />
            <div className="flex flex-1 items-start">
              {hideShares ? null : (
                <div
                  className="w-full rounded-b-sm bg-red-500/60"
                  style={{ height: `${(month.outBtc / peak) * 100}%`, minHeight: month.outBtc > 0 ? 2 : 0 }}
                />
              )}
            </div>
            <span className="pt-0.5 text-center text-2xs text-muted-foreground">{monthLabel(month.date)}</span>
          </div>
        ))}
      </div>
      <dl className="grid grid-cols-3 gap-2 text-xs">
        <div>
          <dt className="text-muted-foreground">{t("widgets.flows.received")}</dt>
          <dd className={cn("font-medium tabular-nums text-emerald-600 dark:text-emerald-400", blurClass(hideSensitive))}>
            {amount(flows.inBtc, flows.inValue)}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">{t("widgets.flows.spent")}</dt>
          <dd className={cn("font-medium tabular-nums text-red-600 dark:text-red-400", blurClass(hideSensitive))}>
            {amount(flows.outBtc, flows.outValue)}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">{t("widgets.flows.fees")}</dt>
          <dd className={cn("font-medium tabular-nums", blurClass(hideSensitive))}>
            {maskSensitive(formatBtc(flows.feeBtc, { precision: 5 }), hideSensitive)}
          </dd>
        </div>
      </dl>
    </WidgetCard>
  );
};
