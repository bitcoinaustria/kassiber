import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { CheckCircle2, TriangleAlert } from "lucide-react";
import { Area, AreaChart, CartesianGrid, Line, Tooltip, XAxis, YAxis } from "recharts";

import { ChartContainer } from "@/components/ui/chart";
import { usePrefersReducedMotion } from "@/hooks/usePrefersReducedMotion";
import { cn } from "@/lib/utils";
import type { OverviewVisual as OverviewVisualModel } from "./model";
import type { VisualFormat } from "./format";
import type { VisualPalette } from "./palette";
import { stagger } from "./motion";
import { CountUp, Legend, Segmented, StatTile, TooltipFrame } from "./primitives";

type Range = "3m" | "1y" | "all";
const RANGE_DAYS = { "3m": 92, "1y": 366 } as const;

interface Props {
  visual: OverviewVisualModel;
  fmt: VisualFormat;
  palette: VisualPalette;
  hideSensitive: boolean;
  expanded?: boolean;
}

export function OverviewVisual({ visual, fmt, palette, hideSensitive, expanded }: Props) {
  const { t } = useTranslation("assistant");
  const reduced = usePrefersReducedMotion();
  const { stats } = visual;
  const incomplete = !stats.basisComplete;
  const unrealizedPct =
    stats.unrealized !== null && stats.costBasis ? stats.unrealized / Math.abs(stats.costBasis) : null;
  const spanDays = visual.series.length > 1
    ? (Date.parse(visual.series[visual.series.length - 1].date) - Date.parse(visual.series[0].date)) / 86_400_000
    : 0;
  const ranges: Range[] = ["all", ...(["1y", "3m"] as const).filter((range) => spanDays > RANGE_DAYS[range] * 1.2)];
  const [range, setRange] = useState<Range>("all");
  const series = useMemo(() => {
    if (range === "all" || !visual.series.length) return visual.series;
    const end = Date.parse(visual.series[visual.series.length - 1].date);
    return visual.series.filter((point) => end - Date.parse(point.date) <= RANGE_DAYS[range] * 86_400_000);
  }, [visual.series, range]);
  const walletColors = [...palette.series, palette.muted];
  const height = expanded ? "h-72" : "h-40";

  return (
    <div className="space-y-3">
      <div className="kb-visual-rise flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <div className="text-2xs text-muted-foreground">{t("visual.overview.portfolioValue")}</div>
          <CountUp
            value={stats.portfolioValue}
            format={fmt.fiat}
            sensitive={hideSensitive}
            className="block text-3xl font-semibold tracking-tight sm:text-4xl"
          />
          <div className="mt-0.5 text-xs text-muted-foreground">
            <CountUp value={stats.btcBalance} format={fmt.btc} sensitive={hideSensitive} />
            {stats.btcPrice !== null ? (
              <span>
                {" · "}
                {t("visual.overview.price", { price: fmt.fiat(stats.btcPrice) })}
                {stats.priceAt ? ` · ${fmt.date(stats.priceAt, "short")}` : ""}
              </span>
            ) : null}
          </div>
        </div>
        <span
          className={cn(
            "inline-flex items-center gap-1 rounded-full px-2 py-0.5 text-2xs font-medium",
            incomplete
              ? "bg-amber-500/10 text-amber-700 dark:text-amber-300"
              : "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
          )}
        >
          {incomplete ? <TriangleAlert className="size-3" aria-hidden="true" /> : <CheckCircle2 className="size-3" aria-hidden="true" />}
          {t(incomplete ? "visual.overview.incomplete" : "visual.overview.complete")}
        </span>
      </div>
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        <StatTile
          label={t("visual.overview.costBasis")}
          value={stats.costBasis}
          format={fmt.fiat}
          incomplete={incomplete}
          incompleteLabel={t("visual.overview.incompleteShort")}
          sensitive={hideSensitive}
          delayMs={stagger(1, 70)}
        />
        <StatTile
          label={t("visual.overview.unrealized")}
          value={stats.unrealized}
          format={(value) => fmt.signed(fmt.fiat(Math.abs(value)), value)}
          delta={!incomplete && unrealizedPct !== null && stats.unrealized !== null ? { label: fmt.percent(unrealizedPct), value: stats.unrealized } : null}
          incomplete={incomplete}
          incompleteLabel={t("visual.overview.incompleteShort")}
          sensitive={hideSensitive}
          delayMs={stagger(2, 70)}
        />
        <StatTile
          label={t("visual.overview.realizedYtd")}
          value={stats.realizedYtd}
          format={(value) => fmt.signed(fmt.fiat(Math.abs(value)), value)}
          incomplete={incomplete}
          incompleteLabel={t("visual.overview.incompleteShort")}
          sensitive={hideSensitive}
          delayMs={stagger(3, 70)}
        />
        <StatTile
          label={t("visual.overview.transactions")}
          value={stats.transactions}
          format={fmt.count}
          delayMs={stagger(4, 70)}
        />
      </div>
      {series.length > 1 ? (
        <div className="kb-visual-rise space-y-1.5" style={{ animationDelay: "240ms" }}>
          <div className="flex items-center justify-between gap-2">
            <Legend
              items={[
                { key: "value", label: t("visual.measure.market_value"), color: palette.series[0] },
                { key: "basis", label: t("visual.measure.cost_basis"), color: palette.muted },
              ]}
            />
            <Segmented
              label={t("visual.controls.range")}
              value={range}
              options={ranges.map((value) => ({ value, label: t(`visual.range.${value}`) }))}
              onChange={setRange}
            />
          </div>
          <ChartContainer config={{}} className={`aspect-auto w-full ${height}`}>
            <AreaChart data={series} margin={{ top: 6, right: 8, bottom: 0, left: 0 }}>
              <defs>
                <linearGradient id={`overview-${visual.callId}`} x1="0" y1="0" x2="0" y2="1">
                  <stop offset="0%" stopColor={palette.series[0]} stopOpacity={0.24} />
                  <stop offset="100%" stopColor={palette.series[0]} stopOpacity={0.02} />
                </linearGradient>
              </defs>
              <CartesianGrid vertical={false} />
              <XAxis dataKey="date" axisLine={false} tickLine={false} tick={{ fontSize: 10 }} minTickGap={28} tickFormatter={(value) => fmt.date(String(value), "month")} />
              <YAxis axisLine={false} tickLine={false} tick={{ fontSize: 10 }} width={58} tickFormatter={(value) => (hideSensitive ? "" : fmt.fiatCompact(Number(value)))} />
              <Tooltip
                isAnimationActive={false}
                cursor={{ stroke: "var(--border)", strokeWidth: 1 }}
                content={({ active, payload, label }) => {
                  if (!active || !payload?.length) return null;
                  const point = payload[0]?.payload as (typeof series)[number] | undefined;
                  if (!point) return null;
                  return (
                    <TooltipFrame
                      sensitive={hideSensitive}
                      title={fmt.date(String(label), "short")}
                      rows={[
                        { key: "v", color: palette.series[0], label: t("visual.measure.market_value"), value: point.marketValue === null ? t("visual.unknownValue") : fmt.fiat(point.marketValue) },
                        { key: "b", color: palette.muted, label: t("visual.measure.cost_basis"), value: point.costBasis === null ? t("visual.unknownValue") : fmt.fiat(point.costBasis) },
                        { key: "q", label: t("visual.measure.quantity"), value: point.balanceBtc === null ? t("visual.unknownValue") : fmt.btc(point.balanceBtc) },
                      ]}
                    />
                  );
                }}
              />
              <Area dataKey="marketValue" type="monotone" stroke={palette.series[0]} strokeWidth={2} fill={`url(#overview-${visual.callId})`} dot={false} activeDot={{ r: 4, strokeWidth: 2, stroke: "var(--card)" }} isAnimationActive={!reduced} animationDuration={1200} animationEasing="ease-out" connectNulls={false} />
              <Line dataKey="costBasis" type="stepAfter" stroke={palette.muted} strokeWidth={1.5} dot={false} isAnimationActive={!reduced} animationDuration={1200} animationBegin={150} />
            </AreaChart>
          </ChartContainer>
        </div>
      ) : null}
      {visual.wallets.length ? (
        <div className="kb-visual-rise space-y-1.5" style={{ animationDelay: "320ms" }}>
          <div className="text-2xs text-muted-foreground">{t("visual.overview.allocation")}</div>
          <div className="flex h-2.5 w-full gap-[2px] overflow-hidden rounded-full" role="img" aria-label={t("visual.overview.allocation")}>
            {visual.wallets.map((wallet, index) =>
              wallet.share > 0 ? (
                <span
                  key={`${wallet.label ?? "other"}-${index}`}
                  className="kb-visual-grow-x h-full origin-left first:rounded-l-full last:rounded-r-full"
                  style={{ width: `${wallet.share * 100}%`, backgroundColor: walletColors[Math.min(index, walletColors.length - 1)], animationDelay: `${360 + index * 60}ms` }}
                />
              ) : null,
            )}
          </div>
          <ul className="grid grid-cols-1 gap-x-4 gap-y-1 text-xs sm:grid-cols-2">
            {visual.wallets.map((wallet, index) => (
              <li key={`${wallet.label ?? "other"}-${index}`} className="flex min-w-0 items-center gap-2">
                <span className="size-2 shrink-0 rounded-xs" style={{ backgroundColor: walletColors[Math.min(index, walletColors.length - 1)] }} aria-hidden="true" />
                <span className="min-w-0 flex-1 truncate">{wallet.label ?? t("visual.overview.otherWallets", { count: wallet.otherCount })}</span>
                <span className={cn("tabular-nums text-muted-foreground", hideSensitive && "sensitive")}>{fmt.btcCompact(wallet.balanceBtc)}</span>
                <span className="w-12 text-right tabular-nums">{fmt.percent(wallet.share)}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  );
}
