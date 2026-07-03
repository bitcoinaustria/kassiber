import { ChevronDown, ChevronUp } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { formatBtc, type Currency } from "@/lib/currency";
import { cn } from "@/lib/utils";

import {
  blurClass,
  formatDetailedPortfolioMoney,
  formatPortfolioMoney,
  type PortfolioChartPoint,
} from "./model";

export function PortfolioInspector({
  point,
  previousPoint,
  hideSensitive,
  priceEur,
  fiatCurrency,
  chartCurrency,
}: {
  point: PortfolioChartPoint | null;
  previousPoint: PortfolioChartPoint | null;
  hideSensitive: boolean;
  priceEur: number;
  fiatCurrency: string;
  chartCurrency: Currency;
}) {
  const { t } = useTranslation("overview");
  // Floating over the chart, the panel must always be dismissable out of the
  // way — collapsing leaves just the header strip.
  const [collapsed, setCollapsed] = React.useState(false);
  const isBtc = chartCurrency === "btc";
  const priorValueEur = previousPoint?.valueEur ?? point?.valueEur ?? 0;
  const pointValueEur = point?.valueEur ?? 0;
  const eurDelta = pointValueEur - priorValueEur;
  const priorBtc = previousPoint?.balanceBtc ?? point?.balanceBtc ?? 0;
  const btcDelta =
    point && previousPoint ? point.balanceBtc - previousPoint.balanceBtc : 0;
  const primaryDelta = isBtc ? btcDelta : eurDelta;
  const primaryPrior = isBtc ? priorBtc : priorValueEur;
  const deltaPct = previousPoint && primaryPrior
    ? (primaryDelta / Math.abs(primaryPrior)) * 100
    : null;
  const secondaryDelta = isBtc ? eurDelta : btcDelta;
  const secondaryLabel = isBtc
    ? `${secondaryDelta >= 0 ? "+" : "−"}${formatPortfolioMoney(
        Math.abs(secondaryDelta),
        priceEur,
        "eur",
        fiatCurrency,
      )}`
    : `${secondaryDelta >= 0 ? "+" : "−"}${formatBtc(
        Math.abs(secondaryDelta),
        { precision: 8 },
      )}`;

  return (
    <aside className="flex max-h-full min-h-0 flex-col gap-3 overflow-y-auto rounded-lg border bg-background/85 p-3 shadow-lg backdrop-blur-sm">
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
            {t("inspector.position")}
          </p>
          {!collapsed && (
            <p className="mt-1 text-sm font-semibold">
              {point?.detailLabel ?? t("inspector.noDateSelected")}
            </p>
          )}
        </div>
        <button
          type="button"
          aria-expanded={!collapsed}
          aria-label={
            collapsed ? t("inspector.expandAria") : t("inspector.collapseAria")
          }
          className="rounded p-0.5 text-muted-foreground transition-colors hover:bg-muted/45 hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() => setCollapsed((current) => !current)}
          onMouseDown={(event) => event.preventDefault()}
        >
          {collapsed ? (
            <ChevronDown className="size-4" aria-hidden="true" />
          ) : (
            <ChevronUp className="size-4" aria-hidden="true" />
          )}
        </button>
      </div>

      {!collapsed && (
        <>
          <div className="grid gap-2">
            <InspectorMetric
              label={t("inspector.value")}
              value={formatPortfolioMoney(
                point?.valueEur ?? 0,
                priceEur,
                "eur",
                fiatCurrency,
              )}
              hidden={hideSensitive}
            />
            <InspectorMetric
              label={t("inspector.btc")}
              value={formatBtc(point?.balanceBtc ?? 0, { precision: 8 })}
              hidden={hideSensitive}
            />
            <InspectorMetric
              label={t("inspector.costBasis")}
              value={formatPortfolioMoney(
                point?.costBasisEur ?? 0,
                priceEur,
                "eur",
                fiatCurrency,
              )}
              hidden={hideSensitive}
            />
            <InspectorMetric
              label={t("inspector.unrealized")}
              value={`${(point?.unrealizedEur ?? 0) >= 0 ? "+ " : "− "}${formatPortfolioMoney(
                Math.abs(point?.unrealizedEur ?? 0),
                priceEur,
                "eur",
                fiatCurrency,
              )}`}
              tone={(point?.unrealizedEur ?? 0) >= 0 ? "good" : "bad"}
              hidden={hideSensitive}
            />
          </div>

          <div className="rounded-md border bg-muted/20 p-2.5">
            <p className="text-[10px] font-medium text-muted-foreground">
              {t("inspector.sincePrevious")}
            </p>
            <div
              className={cn(
                "mt-1 text-sm font-semibold tabular-nums",
                primaryDelta >= 0 ? "text-emerald-600 dark:text-emerald-400" : "text-[var(--kb-accent)]",
                blurClass(hideSensitive),
              )}
            >
              {primaryDelta >= 0 ? "+ " : "− "}
              {formatDetailedPortfolioMoney(
                Math.abs(primaryDelta),
                priceEur,
                chartCurrency,
                fiatCurrency,
              )}
            </div>
            <p className="mt-1 text-[10px] text-muted-foreground">
              {deltaPct === null
                ? t("inspector.startOfRange")
                : `${deltaPct >= 0 ? "+" : "−"}${Math.abs(deltaPct).toFixed(1)}%`}{" "}
              · {secondaryLabel}
            </p>
          </div>
        </>
      )}
    </aside>
  );
}

export function InspectorMetric({
  label,
  value,
  detail,
  tone = "neutral",
  hidden,
}: {
  label: string;
  value: string;
  detail?: string;
  tone?: "good" | "bad" | "neutral";
  hidden: boolean;
}) {
  return (
    <div className="rounded-md bg-muted/25 px-2.5 py-2">
      <p className="text-[10px] font-medium text-muted-foreground">{label}</p>
      <p
        className={cn(
          "mt-0.5 truncate text-sm font-semibold tabular-nums",
          tone === "good" && "text-emerald-600 dark:text-emerald-400",
          tone === "bad" && "text-[var(--kb-accent)]",
          blurClass(hidden),
        )}
      >
        {value}
      </p>
      {detail && (
        <p className="mt-0.5 truncate text-[10px] text-muted-foreground">
          {detail}
        </p>
      )}
    </div>
  );
}
