import { useTranslation } from "react-i18next";

import { formatBtc } from "@/lib/currency";
import { cn } from "@/lib/utils";

import {
  activityFlowColors,
  activityFlowLabelKeys,
  blurClass,
  compactEventId,
  formatFiatPrice,
  formatPortfolioMoney,
  statusLabelKeys,
  type TreasuryChartPoint,
} from "./model";

export interface TreasuryTooltipPayload {
  dataKey?: string | number;
  value?: number | string;
  payload?: TreasuryChartPoint;
}

export interface TreasuryTooltipProps {
  active?: boolean;
  payload?: TreasuryTooltipPayload[];
  label?: string | number;
  activityPointOverride?: TreasuryChartPoint | null;
  hideSensitive: boolean;
  priceEur: number;
  fiatCurrency: string;
  fiatSeriesEnabled?: boolean;
}

export function TreasuryTooltip({
  active,
  payload,
  label,
  activityPointOverride,
  hideSensitive,
  priceEur,
  fiatCurrency,
  fiatSeriesEnabled = true,
}: TreasuryTooltipProps) {
  const { t } = useTranslation("overview");
  if ((!active || !payload?.length) && !activityPointOverride) return null;

  const payloadPoint =
    payload?.find((p) => p.payload?.isActivityEvent)?.payload ??
    payload?.find((p) => p.payload)?.payload;
  const point = activityPointOverride ?? payloadPoint;
  if (!point) return null;

  const unrealizedPct = point.costBasisEur
    ? (point.unrealizedEur / Math.abs(point.costBasisEur)) * 100
    : 0;
  const eventFlow = point.eventFlow;
  const hasEvent = point.isActivityEvent && eventFlow !== undefined;
  const eventTone =
    eventFlow === "incoming" || eventFlow === "swap"
      ? "good"
      : eventFlow === "outgoing" || eventFlow === "fee"
        ? "bad"
        : "neutral";
  const eventAmount =
    eventFlow === "swap"
      ? t("tooltip.volume", {
          value: formatBtc(point.activityBtc, { precision: 8 }),
        })
      : eventFlow === "fee"
        ? formatBtc(-(point.eventFeeBtc || point.activityBtc), {
            precision: 8,
            sign: true,
          })
        : formatBtc(point.eventSignedBtc ?? 0, {
            precision: 8,
            sign: true,
          });
  const eventId = compactEventId(point.eventId);

  if (hasEvent) {
    return (
      <div className="min-w-[280px] max-w-[320px] rounded-lg border border-border bg-popover p-3 text-xs shadow-xl">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-1.5">
              <span
                className="size-2.5 rounded-full"
                style={{ backgroundColor: activityFlowColors[eventFlow] }}
                aria-hidden="true"
              />
              <span className="font-semibold text-foreground">
                {t(activityFlowLabelKeys[eventFlow])}
              </span>
              {point.eventType && (
                <span className="rounded border bg-muted/30 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                  {point.eventType}
                </span>
              )}
            </div>
            <p className="mt-1 text-[10px] text-muted-foreground">
              {point.detailLabel ?? label}
            </p>
          </div>
          <span
            className={cn(
              "shrink-0 text-right font-semibold tabular-nums",
              eventTone === "good" && "text-emerald-500",
              eventTone === "bad" && "text-[var(--kb-accent)]",
              blurClass(hideSensitive),
            )}
          >
            {eventAmount}
          </span>
        </div>

        <div className="mt-3 space-y-1.5">
          {point.eventAccount && (
            <TooltipMetricRow
              label={t("tooltip.source")}
              value={point.eventAccount}
              hidden={false}
            />
          )}
          {point.eventCounter && (
            <TooltipMetricRow
              label={t("tooltip.counterparty")}
              value={point.eventCounter}
              hidden={false}
            />
          )}
          {fiatSeriesEnabled ? (
            <>
              <TooltipMetricRow
                label={t("tooltip.fiatValue")}
                value={formatPortfolioMoney(
                  point.eventFiatValueEur ?? 0,
                  priceEur,
                  "eur",
                  fiatCurrency,
                )}
                hidden={hideSensitive}
              />
              <TooltipMetricRow
                label={t("tooltip.btcPrice")}
                value={formatFiatPrice(point.bitcoinPriceEur, fiatCurrency)}
                hidden={hideSensitive}
              />
            </>
          ) : null}
          {(point.eventFeeBtc ?? 0) > 0 && (
            <TooltipMetricRow
              label={t("tooltip.fee")}
              value={formatBtc(point.eventFeeBtc ?? 0, { precision: 8 })}
              hidden={hideSensitive}
            />
          )}
          <TooltipMetricRow
            label={t("tooltip.positionAfter")}
            value={formatBtc(point.balanceBtc, { precision: 8 })}
            hidden={hideSensitive}
          />
          {fiatSeriesEnabled ? (
            <TooltipMetricRow
              label={t("tooltip.avgBasisAfter")}
              value={
                point.avgCostEur === null
                  ? "—"
                  : formatFiatPrice(point.avgCostEur, fiatCurrency)
              }
              hidden={hideSensitive}
            />
          ) : null}
          <TooltipMetricRow
            label={t("tooltip.status")}
            value={
              point.eventStatus === "confirmed"
                ? t("tooltip.confirmations", {
                    count: point.eventConfirmations ?? 0,
                  })
                : point.eventStatus
                  ? t(statusLabelKeys[point.eventStatus])
                  : t("tooltip.unknown")
            }
            hidden={false}
          />
          {(point.eventTag || eventId) && (
            <div className="flex flex-wrap gap-1.5 pt-1">
              {point.eventTag && (
                <span className="rounded border bg-muted/30 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                  {point.eventTag}
                </span>
              )}
              {eventId && (
                <span className="rounded border bg-muted/30 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
                  {eventId}
                </span>
              )}
            </div>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="min-w-[220px] rounded-lg border border-border bg-popover p-2.5 text-xs shadow-lg">
      <p className="mb-2 font-medium text-foreground">
        {point.detailLabel ?? label}
      </p>
      <div className="space-y-1.5">
        <TooltipMetricRow
          label={t("tooltip.btcBalance")}
          value={formatBtc(point.balanceBtc, { precision: 8 })}
          hidden={hideSensitive}
        />
        {fiatSeriesEnabled ? (
          <>
            <TooltipMetricRow
              label={t("tooltip.btcPrice")}
              value={formatFiatPrice(point.bitcoinPriceEur, fiatCurrency)}
              hidden={hideSensitive}
            />
            <TooltipMetricRow
              label={t("tooltip.avgBasis")}
              value={
                point.avgCostEur === null
                  ? "—"
                  : formatFiatPrice(point.avgCostEur, fiatCurrency)
              }
              hidden={hideSensitive}
            />
            <TooltipMetricRow
              label={t("tooltip.unrealized")}
              value={`${point.unrealizedEur >= 0 ? "+ " : "− "}${formatPortfolioMoney(
                Math.abs(point.unrealizedEur),
                priceEur,
                "eur",
                fiatCurrency,
              )} (${unrealizedPct >= 0 ? "+" : "−"}${Math.abs(unrealizedPct).toFixed(
                1,
              )}%)`}
              tone={point.unrealizedEur >= 0 ? "good" : "bad"}
              hidden={hideSensitive}
            />
          </>
        ) : null}
      </div>
    </div>
  );
}

export function TooltipMetricRow({
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
    <div className="flex items-baseline justify-between gap-4">
      <span className="text-muted-foreground">{label}</span>
      <span
        className={cn(
          "font-medium tabular-nums",
          tone === "good" && "text-emerald-500",
          tone === "bad" && "text-[var(--kb-accent)]",
          blurClass(hidden),
        )}
      >
        {value}
      </span>
    </div>
  );
}
