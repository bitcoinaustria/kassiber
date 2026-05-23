import { formatBtc } from "@/lib/currency";
import { cn } from "@/lib/utils";

import {
  activityFlowColors,
  activityFlowLabels,
  blurClass,
  compactEventId,
  formatEurPrice,
  formatPortfolioMoney,
  statusLabels,
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
  hideSensitive: boolean;
  priceEur: number;
}

export function TreasuryTooltip({
  active,
  payload,
  label,
  hideSensitive,
  priceEur,
}: TreasuryTooltipProps) {
  if (!active || !payload?.length) return null;

  const point =
    payload.find((p) => p.payload?.isActivityEvent)?.payload ??
    payload.find((p) => p.payload)?.payload;
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
      ? `${formatBtc(point.activityBtc, { precision: 8 })} volume`
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
                {activityFlowLabels[eventFlow]}
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
              eventTone === "bad" && "text-[var(--color-accent)]",
              blurClass(hideSensitive),
            )}
          >
            {eventAmount}
          </span>
        </div>

        <div className="mt-3 space-y-1.5">
          {point.eventAccount && (
            <TooltipMetricRow
              label="Source"
              value={point.eventAccount}
              hidden={false}
            />
          )}
          {point.eventCounter && (
            <TooltipMetricRow
              label="Counterparty"
              value={point.eventCounter}
              hidden={false}
            />
          )}
          <TooltipMetricRow
            label="Fiat value"
            value={formatPortfolioMoney(point.eventFiatValueEur ?? 0, priceEur, "eur")}
            hidden={hideSensitive}
          />
          <TooltipMetricRow
            label="BTC price"
            value={formatEurPrice(point.bitcoinPriceEur)}
            hidden={hideSensitive}
          />
          {(point.eventFeeBtc ?? 0) > 0 && (
            <TooltipMetricRow
              label="Fee"
              value={formatBtc(point.eventFeeBtc ?? 0, { precision: 8 })}
              hidden={hideSensitive}
            />
          )}
          <TooltipMetricRow
            label="Position after"
            value={formatBtc(point.balanceBtc, { precision: 8 })}
            hidden={hideSensitive}
          />
          <TooltipMetricRow
            label="Avg basis after"
            value={
              point.avgCostEur === null ? "—" : formatEurPrice(point.avgCostEur)
            }
            hidden={hideSensitive}
          />
          <TooltipMetricRow
            label="Status"
            value={
              point.eventStatus === "confirmed"
                ? `${point.eventConfirmations?.toLocaleString("en-US") ?? 0} confirmations`
                : point.eventStatus
                  ? statusLabels[point.eventStatus]
                  : "Unknown"
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
          label="BTC balance"
          value={formatBtc(point.balanceBtc, { precision: 8 })}
          hidden={hideSensitive}
        />
        <TooltipMetricRow
          label="BTC price"
          value={formatEurPrice(point.bitcoinPriceEur)}
          hidden={hideSensitive}
        />
        <TooltipMetricRow
          label="Avg basis"
          value={
            point.avgCostEur === null ? "—" : formatEurPrice(point.avgCostEur)
          }
          hidden={hideSensitive}
        />
        <TooltipMetricRow
          label="Unrealized"
          value={`${point.unrealizedEur >= 0 ? "+ " : "− "}${formatPortfolioMoney(
            Math.abs(point.unrealizedEur),
            priceEur,
            "eur",
          )} (${unrealizedPct >= 0 ? "+" : "−"}${Math.abs(unrealizedPct).toFixed(
            1,
          )}%)`}
          tone={point.unrealizedEur >= 0 ? "good" : "bad"}
          hidden={hideSensitive}
        />
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
          tone === "bad" && "text-[var(--color-accent)]",
          blurClass(hidden),
        )}
      >
        {value}
      </span>
    </div>
  );
}
