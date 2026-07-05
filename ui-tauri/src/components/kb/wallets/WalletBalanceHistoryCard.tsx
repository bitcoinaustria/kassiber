/**
 * Compact balance-over-time sparkline for the Wallet Detail screen.
 *
 * Reads `ui.reports.balance_history` scoped to a single wallet (the daemon
 * kind already accepts a `wallet` argument) and renders a small area chart.
 * It is an accounting insight — read-only history of the imported balance —
 * not a wallet feature (no spending, no projections).
 */

import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import {
  Area,
  AreaChart,
  ResponsiveContainer,
  Tooltip,
  YAxis,
} from "recharts";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { useDaemon, retryRetryableDaemonError } from "@/daemon/client";
import { localeForLanguage } from "@/i18n/config";
import { cn } from "@/lib/utils";
import {
  portfolioChartColors,
  useResolvedColorMode,
} from "@/components/overview-dashboard/model";

interface BalanceHistoryRow {
  /** Real daemon (report_balance_history) emits `period_start`; older mocks used `bucket`. */
  period_start?: string;
  bucket?: string;
  asset: string;
  quantity: number | string;
}

interface BalanceHistoryData {
  rows: BalanceHistoryRow[];
}

const fmtBtc = (value: number) => `₿ ${value.toFixed(8)}`;

/**
 * Localized short month name for a `YYYY-MM` bucket. Month names follow the UI
 * language (de-AT → „Jän", „Mär", „Dez"), driven by `Intl` rather than a
 * hardcoded English array.
 */
function monthLabel(bucket: string, locale: string) {
  const [year, month] = bucket.split("-");
  const index = Number.parseInt(month ?? "", 10) - 1;
  if (!Number.isInteger(index) || index < 0 || index > 11) {
    return bucket.slice(0, 7);
  }
  const name = new Intl.DateTimeFormat(locale, { month: "short" }).format(
    new Date(Date.UTC(2021, index, 15)),
  );
  return `${name} ${year}`;
}

export function WalletBalanceHistoryCard({
  walletId,
  hideSensitive,
}: {
  walletId: string;
  hideSensitive: boolean;
}) {
  const { t, i18n } = useTranslation("connections");
  const locale = localeForLanguage(i18n.language);
  const colorMode = useResolvedColorMode();
  const color = portfolioChartColors[colorMode].value;
  const gradientId = `wallet-balance-${walletId}`;
  const query = useDaemon<BalanceHistoryData>(
    "ui.reports.balance_history",
    { wallet: walletId, interval: "month", limit: 24 },
    { retry: retryRetryableDaemonError },
  );

  const points = useMemo(() => {
    const rows = query.data?.data?.rows ?? [];
    // balance_history returns one row per asset per period. Aggregate the
    // BTC-denominated assets by bucket so a wallet with both BTC and L-BTC
    // activity gets one point per period (and `latest`/`change` reflect the
    // total, not whichever asset row happened to be last).
    const byBucket = new Map<string, number>();
    for (const row of rows) {
      const asset = (row.asset ?? "").toUpperCase();
      if (asset !== "BTC" && asset !== "LBTC" && asset !== "L-BTC") continue;
      const period = row.period_start ?? row.bucket;
      if (!period) continue;
      byBucket.set(
        period,
        (byBucket.get(period) ?? 0) + (Number(row.quantity) || 0),
      );
    }
    return Array.from(byBucket.entries())
      .sort((a, b) => a[0].localeCompare(b[0]))
      .map(([bucket, quantity]) => ({ bucket, quantity }));
  }, [query.data]);

  // Nothing meaningful to plot — keep the screen uncluttered for sources with
  // no scoped history (e.g. just-added wallets, or a backend that errored).
  if (!query.isLoading && points.length < 2) {
    return null;
  }

  const latest = points.length ? points[points.length - 1].quantity : 0;
  const first = points.length ? points[0].quantity : 0;
  const change = latest - first;

  return (
    <Card className="gap-0 overflow-hidden py-0 shadow-none">
      <CardHeader className="border-b p-3 sm:px-6 sm:py-3.5 [.border-b]:pb-3 sm:[.border-b]:pb-3.5">
        <CardTitle className="text-sm sm:text-base">
          {t("detail.balanceHistory.title")}
        </CardTitle>
        <CardDescription className="text-xs">
          {t("detail.balanceHistory.description", {
            count: points.length || 12,
          })}
        </CardDescription>
      </CardHeader>
      <CardContent className="px-3 py-3 sm:px-6">
        {query.isLoading ? (
          <Skeleton className="h-24 w-full" />
        ) : (
          <>
            <div className="flex items-baseline justify-between gap-2">
              <span
                className={cn(
                  "font-mono text-base font-semibold tabular-nums",
                  hideSensitive && "sensitive",
                )}
              >
                {fmtBtc(latest)}
              </span>
              <span
                className={cn(
                  "font-mono text-xs tabular-nums",
                  change > 0
                    ? "text-emerald-600 dark:text-emerald-400"
                    : change < 0
                      ? "text-red-600 dark:text-red-400"
                      : "text-muted-foreground",
                  hideSensitive && "sensitive",
                )}
              >
                {change >= 0 ? "+" : "−"}
                {fmtBtc(Math.abs(change))}
              </span>
            </div>
            <div className="mt-2 h-24 w-full">
              <ResponsiveContainer width="100%" height="100%">
                <AreaChart
                  data={points}
                  margin={{ top: 4, right: 0, bottom: 0, left: 0 }}
                >
                  <defs>
                    <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
                      <stop offset="0%" stopColor={color} stopOpacity={0.3} />
                      <stop offset="100%" stopColor={color} stopOpacity={0.02} />
                    </linearGradient>
                  </defs>
                  <YAxis hide domain={["dataMin", "dataMax"]} />
                  <Tooltip
                    cursor={{ stroke: "var(--border)", strokeWidth: 1 }}
                    isAnimationActive={false}
                    content={({ active, payload }) => {
                      if (!active || !payload?.length) return null;
                      const point = payload[0]?.payload as
                        | { bucket: string; quantity: number }
                        | undefined;
                      if (!point) return null;
                      return (
                        <div className="rounded-md border bg-popover px-2 py-1 text-xs shadow-sm">
                          <div className="text-muted-foreground">
                            {monthLabel(point.bucket, locale)}
                          </div>
                          <div
                            className={cn(
                              "font-mono font-medium tabular-nums",
                              hideSensitive && "sensitive",
                            )}
                          >
                            {fmtBtc(point.quantity)}
                          </div>
                        </div>
                      );
                    }}
                  />
                  <Area
                    type="monotone"
                    dataKey="quantity"
                    stroke={color}
                    strokeWidth={1.75}
                    fill={`url(#${gradientId})`}
                    fillOpacity={1}
                    dot={false}
                    activeDot={{ r: 3, fill: color, strokeWidth: 0 }}
                    isAnimationActive={false}
                  />
                </AreaChart>
              </ResponsiveContainer>
            </div>
          </>
        )}
      </CardContent>
    </Card>
  );
}
