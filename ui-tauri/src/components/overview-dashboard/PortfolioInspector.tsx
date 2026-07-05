import { ChevronDown, ChevronUp } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";

import { formatBtc } from "@/lib/currency";
import { cn } from "@/lib/utils";

import {
  blurClass,
  formatPortfolioMoney,
  type PortfolioChartPoint,
} from "./model";

export function PortfolioInspector({
  point,
  hideSensitive,
  priceEur,
  fiatCurrency,
  variant = "panel",
  className,
}: {
  point: PortfolioChartPoint | null;
  hideSensitive: boolean;
  priceEur: number;
  fiatCurrency: string;
  variant?: "panel" | "header";
  className?: string;
}) {
  const { t } = useTranslation("overview");
  // The legacy panel layout remains collapsible; the expanded chart header
  // uses the compact variant below.
  const [collapsed, setCollapsed] = React.useState(false);
  const valueLabel = formatPortfolioMoney(
    point?.valueEur ?? 0,
    priceEur,
    "eur",
    fiatCurrency,
  );
  const btcLabel = formatBtc(point?.balanceBtc ?? 0, { precision: 8 });
  const costBasisLabel = formatPortfolioMoney(
    point?.costBasisEur ?? 0,
    priceEur,
    "eur",
    fiatCurrency,
  );
  const unrealizedLabel = `${(point?.unrealizedEur ?? 0) >= 0 ? "+ " : "− "}${formatPortfolioMoney(
    Math.abs(point?.unrealizedEur ?? 0),
    priceEur,
    "eur",
    fiatCurrency,
  )}`;

  if (variant === "header") {
    return (
      <aside
        className={cn(
          "flex min-w-0 max-w-full flex-wrap items-center gap-1.5 rounded-md border bg-background/85 px-2 py-1.5 text-xs shadow-sm backdrop-blur-sm",
          className,
        )}
      >
        <div className="min-w-[88px] pr-1">
          <p className="text-[9px] font-medium tracking-wide text-muted-foreground uppercase">
            {t("inspector.position")}
          </p>
          <p className="truncate text-[11px] font-semibold">
            {point?.detailLabel ?? t("inspector.noDateSelected")}
          </p>
        </div>
        <HeaderInspectorMetric
          label={t("inspector.value")}
          value={valueLabel}
          hidden={hideSensitive}
        />
        <HeaderInspectorMetric
          label={t("inspector.btc")}
          value={btcLabel}
          hidden={hideSensitive}
        />
        <HeaderInspectorMetric
          label={t("inspector.costBasis")}
          value={costBasisLabel}
          hidden={hideSensitive}
        />
        <HeaderInspectorMetric
          label={t("inspector.unrealized")}
          value={unrealizedLabel}
          tone={(point?.unrealizedEur ?? 0) >= 0 ? "good" : "bad"}
          hidden={hideSensitive}
        />
      </aside>
    );
  }

  return (
    <aside
      className={cn(
        "flex max-h-full min-h-0 flex-col gap-3 overflow-y-auto rounded-lg border bg-background/85 p-3 shadow-lg backdrop-blur-sm",
        className,
      )}
    >
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
              value={valueLabel}
              hidden={hideSensitive}
            />
            <InspectorMetric
              label={t("inspector.btc")}
              value={btcLabel}
              hidden={hideSensitive}
            />
            <InspectorMetric
              label={t("inspector.costBasis")}
              value={costBasisLabel}
              hidden={hideSensitive}
            />
            <InspectorMetric
              label={t("inspector.unrealized")}
              value={unrealizedLabel}
              tone={(point?.unrealizedEur ?? 0) >= 0 ? "good" : "bad"}
              hidden={hideSensitive}
            />
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

function HeaderInspectorMetric({
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
    <div className="min-w-[92px] rounded bg-muted/20 px-2 py-1">
      <p className="truncate text-[9px] font-medium text-muted-foreground">
        {label}
      </p>
      <p
        className={cn(
          "truncate text-[11px] font-semibold tabular-nums",
          tone === "good" && "text-emerald-600 dark:text-emerald-400",
          tone === "bad" && "text-[var(--kb-accent)]",
          blurClass(hidden),
        )}
      >
        {value}
      </p>
      {detail ? (
        <p className="truncate text-[9px] text-muted-foreground">{detail}</p>
      ) : null}
    </div>
  );
}
