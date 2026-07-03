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
    <aside className="flex min-h-0 flex-col gap-3 rounded-lg border bg-background/65 p-3 xl:max-h-[min(64vh,620px)] xl:overflow-y-auto">
      <div>
        <p className="text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
          {t("inspector.selectedPoint")}
        </p>
        <p className="mt-1 text-sm font-semibold">
          {point?.detailLabel ?? t("inspector.noPointSelected")}
        </p>
      </div>

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
    </aside>
  );
}

export function InspectorMetric({
  label,
  value,
  tone = "neutral",
  hidden,
}: {
  label: string;
  value: string;
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
    </div>
  );
}
