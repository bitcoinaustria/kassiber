import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { ChevronRight } from "lucide-react";
import { useTranslation } from "react-i18next";

import { formatBtc, formatFiatAmount, formatWholeFiatAmount, type Currency } from "@/lib/currency";
import { fiatCompleteness } from "@/lib/fiatCompleteness";
import { cn } from "@/lib/utils";
import type { OverviewSnapshot } from "@/mocks/seed";

import { activeMarketFiatCurrency, activityTxs, blurClass } from "./model";

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
  const year = new Date().getFullYear();
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
              {row.value}
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

const MONTHS = 12;

/**
 * Money in and out per month over the last year, from the same flow reading
 * as the chart's activity dots: received above the line, spent below, moves
 * between your own wallets left out. Totals and fees underneath.
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
  const fiatCurrency = activeMarketFiatCurrency(snapshot);
  const now = new Date();
  const months = Array.from({ length: MONTHS }, (_, index) => {
    const date = new Date(now.getFullYear(), now.getMonth() - (MONTHS - 1 - index), 1);
    return { key: `${date.getFullYear()}-${date.getMonth()}`, date, inBtc: 0, outBtc: 0, inValue: 0, outValue: 0 };
  });
  const byKey = new Map(months.map((month) => [month.key, month]));
  let feeBtc = 0;
  for (const event of activityTxs(snapshot)) {
    const month = byKey.get(`${event.occurredAt.getFullYear()}-${event.occurredAt.getMonth()}`);
    if (!month) continue;
    feeBtc += event.feeBtc;
    if (event.flow === "incoming") {
      month.inBtc += event.btc;
      month.inValue += event.valueEur;
    } else if (event.flow === "outgoing") {
      month.outBtc += event.btc;
      month.outValue += event.valueEur;
    }
  }
  const isBitcoinMode = currency === "btc";
  const inBtc = months.reduce((sum, month) => sum + month.inBtc, 0);
  const outBtc = months.reduce((sum, month) => sum + month.outBtc, 0);
  const inValue = months.reduce((sum, month) => sum + month.inValue, 0);
  const outValue = months.reduce((sum, month) => sum + month.outValue, 0);
  const peak = Math.max(...months.map((month) => Math.max(month.inBtc, month.outBtc)), 0) || 1;
  const amount = (btc: number, value: number) =>
    isBitcoinMode ? formatBtc(btc, { precision: 4 }) : formatWholeFiatAmount(value, fiatCurrency);
  const monthLabel = (date: Date) => date.toLocaleDateString(i18n.language, { month: "narrow" });
  return (
    <WidgetCard title={t("widgets.flows.title")}>
      <div
        className={cn("grid h-28 grid-cols-12 items-stretch gap-1", blurClass(hideSensitive))}
        role="img"
        aria-label={t("widgets.flows.chartAria")}
      >
        {months.map((month) => (
          <div
            key={month.key}
            className="flex min-w-0 flex-col"
            title={`${month.date.toLocaleDateString(i18n.language, { month: "long", year: "numeric" })}: ${t("widgets.flows.received")} ${formatBtc(month.inBtc, { precision: 4 })} · ${t("widgets.flows.spent")} ${formatBtc(month.outBtc, { precision: 4 })}`}
          >
            <div className="flex flex-1 items-end">
              <div
                className="w-full rounded-t-sm bg-emerald-500/70"
                style={{ height: `${(month.inBtc / peak) * 100}%`, minHeight: month.inBtc > 0 ? 2 : 0 }}
              />
            </div>
            <div className="h-px bg-border" />
            <div className="flex flex-1 items-start">
              <div
                className="w-full rounded-b-sm bg-red-500/60"
                style={{ height: `${(month.outBtc / peak) * 100}%`, minHeight: month.outBtc > 0 ? 2 : 0 }}
              />
            </div>
            <span className="pt-0.5 text-center text-2xs text-muted-foreground">{monthLabel(month.date)}</span>
          </div>
        ))}
      </div>
      <dl className="grid grid-cols-3 gap-2 text-xs">
        <div>
          <dt className="text-muted-foreground">{t("widgets.flows.received")}</dt>
          <dd className={cn("font-medium tabular-nums text-emerald-600 dark:text-emerald-400", blurClass(hideSensitive))}>
            {amount(inBtc, inValue)}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">{t("widgets.flows.spent")}</dt>
          <dd className={cn("font-medium tabular-nums text-red-600 dark:text-red-400", blurClass(hideSensitive))}>
            {amount(outBtc, outValue)}
          </dd>
        </div>
        <div>
          <dt className="text-muted-foreground">{t("widgets.flows.fees")}</dt>
          <dd className={cn("font-medium tabular-nums", blurClass(hideSensitive))}>
            {formatBtc(feeBtc, { precision: 5 })}
          </dd>
        </div>
      </dl>
    </WidgetCard>
  );
};
