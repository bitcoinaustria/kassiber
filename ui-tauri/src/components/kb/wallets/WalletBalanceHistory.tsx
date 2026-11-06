/**
 * Balance-over-time for the Wallet Detail hero.
 *
 * `useWalletBalanceHistory` reads `ui.reports.balance_history` scoped to one
 * wallet and folds it into monthly points; `WalletBalanceHistory` draws them
 * with the Overview chart's axes, colours and step line, without a card of its
 * own, so the hero can sit it beside the balance. It follows the app-wide
 * BTC/fiat switch: fiat mode plots the daemon's month-end market value. It is
 * an accounting insight — read-only history of the booked balance — not a
 * wallet feature (no spending, no projections).
 */

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Area,
  AreaChart,
  CartesianGrid,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

import { ChartContainer, type ChartConfig } from "@/components/ui/chart";
import { Skeleton } from "@/components/ui/skeleton";
import { useDaemon, retryRetryableDaemonError } from "@/daemon/client";
import { localeForLanguage } from "@/i18n/config";
import {
  formatBtc,
  formatCompactFiatAmount,
  formatFiatAmount,
  type Currency,
} from "@/lib/currency";
import { cn } from "@/lib/utils";
import {
  formatBtcAxis,
  portfolioChartColors,
  useResolvedColorMode,
} from "@/components/overview-dashboard/model";

interface BalanceHistoryRow {
  /** Real daemon (report_balance_history) emits `period_start`; older mocks used `bucket`. */
  period_start?: string;
  bucket?: string;
  asset: string;
  quantity: number | string;
  market_value?: number | string | null;
}

interface BalanceHistoryData {
  rows: BalanceHistoryRow[];
  summary?: { truncated?: boolean };
}

export interface BalancePoint {
  bucket: string;
  quantity: number;
  /** Month-end market value; null when any asset row in the month lacks a price. */
  value: number | null;
}

type BalanceRange = "1y" | "3y" | "all";

const RANGE_MONTHS: Record<BalanceRange, number | null> = {
  "1y": 12,
  "3y": 36,
  all: null,
};

// The daemon's cap. It counts rows, one per asset per month, so a wallet
// holding one asset gets over 40 years before the oldest months drop.
const HISTORY_ROW_LIMIT = 500;

const BTC_ASSETS = new Set(["BTC", "LBTC", "L-BTC"]);

function bucketDate(bucket: string) {
  const [year, month] = bucket.split("-");
  return new Date(
    Date.UTC(Number.parseInt(year ?? "", 10), Number.parseInt(month ?? "", 10) - 1, 15),
  );
}

function shiftMonth(bucket: string, months: number) {
  const date = bucketDate(bucket);
  if (Number.isNaN(date.getTime())) return bucket;
  date.setUTCMonth(date.getUTCMonth() + months);
  return `${date.getUTCFullYear()}-${String(date.getUTCMonth() + 1).padStart(2, "0")}`;
}

/**
 * One point per month from the wallet's first booked month to `now`'s, with
 * its BTC-denominated assets (BTC and L-BTC) summed.
 *
 * The daemon leaves out months the wallet held nothing in, so those come back
 * as zero. Its market value is the quantity times the latest known rate, which
 * is zero before any price, so a held month valued at zero has no price. When
 * the daemon dropped the oldest rows, the oldest month left may be missing an
 * asset, so it is dropped too and `complete` says the history no longer
 * starts at the wallet's first booking.
 */
export function balancePointsFromRows(
  rows: BalanceHistoryRow[],
  { truncated = false, now = new Date() }: { truncated?: boolean; now?: Date } = {},
): { points: BalancePoint[]; complete: boolean } {
  const byBucket = new Map<string, BalancePoint>();
  for (const row of rows) {
    if (!BTC_ASSETS.has((row.asset ?? "").toUpperCase())) continue;
    const period = row.period_start ?? row.bucket;
    if (!period) continue;
    const bucket = period.slice(0, 7);
    const quantity = Number(row.quantity) || 0;
    const marketValue =
      row.market_value === null || row.market_value === undefined
        ? null
        : Number(row.market_value);
    const priced =
      marketValue !== null &&
      Number.isFinite(marketValue) &&
      (marketValue !== 0 || quantity === 0);
    const current = byBucket.get(bucket) ?? { bucket, quantity: 0, value: 0 };
    byBucket.set(bucket, {
      bucket,
      quantity: current.quantity + quantity,
      value:
        current.value === null || !priced ? null : current.value + marketValue,
    });
  }
  const booked = Array.from(byBucket.keys()).sort();
  if (truncated) booked.shift();
  if (!booked.length) return { points: [], complete: !truncated };
  const nowBucket = `${now.getUTCFullYear()}-${String(now.getUTCMonth() + 1).padStart(2, "0")}`;
  const lastBucket =
    booked[booked.length - 1] > nowBucket ? booked[booked.length - 1] : nowBucket;
  const points: BalancePoint[] = [];
  for (
    let bucket = booked[0];
    bucket <= lastBucket && points.length < 1200;
    bucket = shiftMonth(bucket, 1)
  ) {
    points.push(byBucket.get(bucket) ?? { bucket, quantity: 0, value: 0 });
  }
  return { points, complete: !truncated };
}

/** Localized `MMM YYYY` for a `YYYY-MM` bucket (de-AT → „Jän 2024"). */
function monthLabel(bucket: string, locale: string) {
  const date = bucketDate(bucket);
  if (Number.isNaN(date.getTime())) return bucket.slice(0, 7);
  return new Intl.DateTimeFormat(locale, {
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  }).format(date);
}

/** Axis tick, as on the Overview chart: `Okt 21`. */
function monthTick(bucket: string, locale: string) {
  const date = bucketDate(bucket);
  if (Number.isNaN(date.getTime())) return bucket.slice(0, 7);
  const month = new Intl.DateTimeFormat(locale, {
    month: "short",
    timeZone: "UTC",
  }).format(date);
  return `${month} ${String(date.getUTCFullYear()).slice(-2)}`;
}

export function useWalletBalanceHistory(walletId: string) {
  const query = useDaemon<BalanceHistoryData>(
    "ui.reports.balance_history",
    { wallet: walletId, interval: "month", limit: HISTORY_ROW_LIMIT },
    { retry: retryRetryableDaemonError },
  );
  const { points, complete } = useMemo(
    () =>
      balancePointsFromRows(query.data?.data?.rows ?? [], {
        truncated: Boolean(query.data?.data?.summary?.truncated),
      }),
    [query.data],
  );
  return {
    points,
    complete,
    isLoading: query.isLoading,
    // A single booked month has no history to draw (just-added sources, or a
    // backend that errored).
    hasHistory: query.isLoading || points.length >= 2,
  };
}

export function WalletBalanceHistory({
  walletId,
  points,
  complete,
  isLoading,
  hideSensitive,
  currency,
  fiatCurrency,
}: {
  walletId: string;
  points: BalancePoint[];
  /** The points start at the wallet's first booked month. */
  complete: boolean;
  isLoading: boolean;
  hideSensitive: boolean;
  currency: Currency;
  fiatCurrency: string;
}) {
  const { t, i18n } = useTranslation("connections");
  const locale = localeForLanguage(i18n.language);
  const colorMode = useResolvedColorMode();
  const [range, setRange] = useState<BalanceRange>("all");
  const gradientId = `wallet-balance-${walletId}`;

  // Each point is a month-end balance, so a range's change is measured from
  // the month before it: the previous bucket, or zero before the wallet's
  // first booked month. That baseline also leads the line, so the step into
  // the first month is drawn instead of hidden. A history cut short has no
  // known month before it and measures from its first point.
  const rangeMonths = RANGE_MONTHS[range];
  const rangeStart =
    rangeMonths === null ? 0 : Math.max(0, points.length - rangeMonths);
  const rangePoints = points.slice(rangeStart);
  const baselinePoint: BalancePoint | null =
    rangeStart > 0
      ? points[rangeStart - 1]
      : points.length && complete
        ? { bucket: shiftMonth(points[0].bucket, -1), quantity: 0, value: 0 }
        : null;
  const visiblePoints = baselinePoint
    ? [baselinePoint, ...rangePoints]
    : rangePoints;
  // Fiat mode only when every visible month has a market value; one unpriced
  // month would otherwise draw as a cliff to zero.
  const showFiat =
    currency === "eur" &&
    visiblePoints.length > 0 &&
    visiblePoints.every((point) => point.value !== null);
  const valueOf = (point: BalancePoint) =>
    showFiat ? (point.value ?? 0) : point.quantity;
  const formatValue = (value: number, options?: { sign?: boolean }) => {
    if (!showFiat) return formatBtc(value, { sign: options?.sign });
    const formatted = formatFiatAmount(Math.abs(value), fiatCurrency);
    if (!options?.sign) return formatted;
    return `${value < 0 ? "− " : "+ "}${formatted}`;
  };
  const color = showFiat
    ? portfolioChartColors[colorMode].portfolioValue
    : portfolioChartColors[colorMode].value;
  const chartConfig = {
    balance: { label: t("detail.balanceHistory.title"), color },
  } satisfies ChartConfig;

  if (!isLoading && points.length < 2) return null;

  const first = rangePoints[0];
  const latest = rangePoints[rangePoints.length - 1];
  const changeFrom = baselinePoint ?? first;
  const change =
    changeFrom && latest ? valueOf(latest) - valueOf(changeFrom) : 0;
  const rangeOptions = (Object.keys(RANGE_MONTHS) as BalanceRange[]).filter(
    // Offer a window only when it actually cuts the history.
    (option) => {
      const months = RANGE_MONTHS[option];
      return months === null || points.length > months;
    },
  );

  return (
    <div className="flex min-w-0 flex-col">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <span className="text-sm font-medium">
              {t("detail.balanceHistory.title")}
            </span>
            {latest ? (
              <span className="text-2xs text-muted-foreground sm:text-xs">
                {t("detail.balanceHistory.asOf", {
                  month: monthLabel(latest.bucket, locale),
                })}
              </span>
            ) : null}
          </div>
          {!isLoading && first ? (
            <p className="mt-1 text-xs text-muted-foreground">
              <span
                className={cn(
                  "font-medium tabular-nums",
                  change > 0
                    ? "text-emerald-600 dark:text-emerald-400"
                    : change < 0
                      ? "text-red-600 dark:text-red-400"
                      : "text-muted-foreground",
                  hideSensitive && "sensitive",
                )}
              >
                {formatValue(change, { sign: true })}
              </span>{" "}
              {t("detail.balanceHistory.since", {
                month: monthLabel(first.bucket, locale),
              })}
            </p>
          ) : null}
        </div>
        {rangeOptions.length > 1 ? (
          <div
            className="flex items-center gap-0.5"
            role="group"
            aria-label={t("detail.balanceHistory.rangeLabel")}
          >
            {rangeOptions.map((option) => (
              <button
                key={option}
                type="button"
                aria-pressed={range === option}
                onClick={() => setRange(option)}
                className={cn(
                  "rounded-md px-2 py-1 text-xs font-medium text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  range === option && "bg-muted text-foreground",
                )}
              >
                {/* dynamic key */}
                {t(`detail.balanceHistory.range.${option}` as never)}
              </button>
            ))}
          </div>
        ) : null}
      </div>
      <div className="-mx-1 pt-2">
        {isLoading ? (
          <Skeleton className="mx-1 h-52 rounded-lg" />
        ) : (
          <ChartContainer
            config={chartConfig}
            className="aspect-auto h-52 w-full"
          >
            <AreaChart
              accessibilityLayer={false}
              data={visiblePoints}
              margin={{ top: 8, right: 12, bottom: 4, left: 4 }}
            >
              <defs>
                <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor={color} stopOpacity={0.24} />
                  <stop offset="100%" stopColor={color} stopOpacity={0.02} />
                </linearGradient>
              </defs>
              <CartesianGrid vertical={false} strokeOpacity={0.45} />
              <XAxis
                dataKey="bucket"
                axisLine={false}
                tickLine={false}
                tick={{ fontSize: 10 }}
                dy={6}
                minTickGap={28}
                tickFormatter={(value) => monthTick(String(value), locale)}
              />
              <YAxis
                axisLine={false}
                tickLine={false}
                tick={{ fontSize: 10 }}
                tickMargin={4}
                width={64}
                domain={[0, "auto"]}
                tickFormatter={(value) =>
                  hideSensitive
                    ? ""
                    : showFiat
                      ? formatCompactFiatAmount(Number(value), fiatCurrency)
                      : formatBtcAxis(Number(value))
                }
              />
              <Tooltip
                cursor={{ stroke: "var(--border)", strokeWidth: 1 }}
                isAnimationActive={false}
                content={({ active, payload }) => {
                  if (!active || !payload?.length) return null;
                  const point = payload[0]?.payload as BalancePoint | undefined;
                  if (!point) return null;
                  return (
                    <div className="rounded-md border bg-popover px-2.5 py-1.5 text-xs shadow-sm">
                      <div className="text-muted-foreground">
                        {monthLabel(point.bucket, locale)}
                      </div>
                      <div
                        className={cn(
                          "font-medium tabular-nums",
                          hideSensitive && "sensitive",
                        )}
                      >
                        {formatBtc(point.quantity)}
                      </div>
                      {point.value !== null ? (
                        <div
                          className={cn(
                            "text-muted-foreground tabular-nums",
                            hideSensitive && "sensitive",
                          )}
                        >
                          {formatFiatAmount(point.value, fiatCurrency)}
                        </div>
                      ) : null}
                    </div>
                  );
                }}
              />
              <Area
                type="stepAfter"
                dataKey={showFiat ? "value" : "quantity"}
                stroke={color}
                strokeWidth={1.75}
                fill={`url(#${gradientId})`}
                fillOpacity={1}
                dot={false}
                activeDot={{ r: 3, fill: color, strokeWidth: 0 }}
                isAnimationActive={false}
              />
            </AreaChart>
          </ChartContainer>
        )}
      </div>
    </div>
  );
}
