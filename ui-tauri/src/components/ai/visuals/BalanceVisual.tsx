import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Area, AreaChart, CartesianGrid, Line, Tooltip, XAxis, YAxis } from "recharts";

import { ChartContainer, type ChartConfig } from "@/components/ui/chart";
import { usePrefersReducedMotion } from "@/hooks/usePrefersReducedMotion";
import type { BalanceVisual as BalanceVisualModel } from "./model";
import type { VisualFormat } from "./format";
import { seriesColorMap, type VisualPalette } from "./palette";
import { CountUp, DeltaPill, Legend, Segmented, TooltipFrame } from "./primitives";

type Measure = "quantity" | "market_value" | "cost_basis" | "compare";
type Range = "3m" | "1y" | "all";

const RANGE_DAYS: Record<Exclude<Range, "all">, number> = { "3m": 92, "1y": 366 };

interface Props {
  visual: BalanceVisualModel;
  fmt: VisualFormat;
  palette: VisualPalette;
  hideSensitive: boolean;
  expanded?: boolean;
}

export function BalanceVisual({ visual, fmt, palette, hideSensitive, expanded }: Props) {
  const { t } = useTranslation("assistant");
  const reduced = usePrefersReducedMotion();
  const assets = useMemo(() => [...new Set(visual.rows.map((row) => row.asset))], [visual.rows]);
  const single = assets.length === 1;
  const [measure, setMeasure] = useState<Measure>(() => {
    const initial = visual.measure as Measure;
    return initial === "market_value" && single ? "compare" : initial;
  });
  const periods = useMemo(() => [...new Set(visual.rows.map((row) => row.period))], [visual.rows]);
  const spanDays = periods.length > 1
    ? (Date.parse(periods[periods.length - 1]) - Date.parse(periods[0])) / 86_400_000
    : 0;
  const ranges: Range[] = ["all", ...(["1y", "3m"] as const).filter((range) => spanDays > RANGE_DAYS[range] * 1.2)];
  const [range, setRange] = useState<Range>("all");
  const colors = seriesColorMap(assets, palette);

  const visiblePeriods = useMemo(() => {
    if (range === "all" || periods.length === 0) return periods;
    const end = Date.parse(periods[periods.length - 1]);
    return periods.filter((period) => end - Date.parse(period) <= RANGE_DAYS[range] * 86_400_000);
  }, [periods, range]);

  const valueKey = measure === "quantity" ? "quantity" : measure === "cost_basis" ? "costBasis" : "marketValue";
  const fiat = measure !== "quantity";
  const keys = useMemo(() => assets.map((asset, index) => ({ asset, key: `s${index}` })), [assets]);
  const data = useMemo(() => {
    const byPeriod = new Map<string, Record<string, number | string | null>>();
    for (const period of visiblePeriods) byPeriod.set(period, { x: period });
    for (const row of visual.rows) {
      const point = byPeriod.get(row.period);
      if (!point) continue;
      const key = keys.find((item) => item.asset === row.asset)?.key ?? "s0";
      point[key] = row[valueKey];
      if (measure === "compare") point.basis = row.costBasis;
    }
    return [...byPeriod.values()];
  }, [visual.rows, visiblePeriods, valueKey, measure, keys]);

  const format = (value: number, asset: string) => (fiat ? fmt.fiat(value) : fmt.asset(value, asset));
  // The headline: one asset's own figure; for fiat measures the total of all
  // assets (one currency); amounts of different assets are not added up.
  const pointTotal = (point: Record<string, number | string | null> | undefined) => {
    if (!point) return null;
    if (single) return typeof point.s0 === "number" ? point.s0 : null;
    if (!fiat) return null;
    let total = 0;
    for (const { key } of keys) {
      const value = point[key];
      if (typeof value !== "number") return null;
      total += value;
    }
    return total;
  };
  const first = pointTotal(data[0]);
  const last = pointTotal(data[data.length - 1]);
  const change = first !== null && last !== null ? last - first : null;
  const config = Object.fromEntries(
    keys.map(({ asset, key }) => [key, { label: asset, color: colors.get(asset) }]),
  ) as ChartConfig;
  const measureOptions = [
    { value: "quantity" as const, label: t("visual.measure.quantity") },
    ...(single
      ? [{ value: "compare" as const, label: t("visual.measure.compare") }]
      : [{ value: "market_value" as const, label: t("visual.measure.market_value") }]),
    { value: "cost_basis" as const, label: t("visual.measure.cost_basis") },
  ];
  const style = visual.interval === "month" ? "month" : visual.interval === "hour" ? "hour" : "day";
  const height = expanded ? "h-80" : "h-48";

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <div className="text-2xs text-muted-foreground">
            {t("visual.balance.latest", { date: fmt.date(visiblePeriods[visiblePeriods.length - 1] ?? "", style) })}
          </div>
          <div className="flex flex-wrap items-baseline gap-2">
            {single || fiat ? (
              <CountUp
                value={last}
                format={(value) => format(value, assets[0] ?? "BTC")}
                sensitive={hideSensitive}
                className="text-2xl font-semibold tracking-tight"
              />
            ) : (
              <div className="text-sm text-muted-foreground">{t("visual.allocation.mixedAssets")}</div>
            )}
            {change !== null ? (
              <DeltaPill
                value={change}
                sensitive={hideSensitive}
                // An absolute change since the first bucket; a percentage of
                // a balance that started near zero says nothing.
                label={t("visual.balance.change", {
                  change: fmt.signed(format(Math.abs(change), assets[0]), change),
                  date: fmt.date(visiblePeriods[0] ?? "", style),
                })}
              />
            ) : null}
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Segmented label={t("visual.controls.measure")} value={measure} options={measureOptions} onChange={setMeasure} />
          <Segmented
            label={t("visual.controls.range")}
            value={range}
            options={ranges.map((value) => ({ value, label: t(`visual.range.${value}`) }))}
            onChange={setRange}
          />
        </div>
      </div>
      <ChartContainer config={config} className={`aspect-auto w-full ${height}`}>
        <AreaChart data={data} margin={{ top: 8, right: 12, bottom: 0, left: 0 }}>
          <defs>
            {keys.map(({ asset, key }) => (
              <linearGradient key={key} id={`fill-${visual.callId}-${key}`} x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor={colors.get(asset)} stopOpacity={0.22} />
                <stop offset="100%" stopColor={colors.get(asset)} stopOpacity={0.02} />
              </linearGradient>
            ))}
          </defs>
          <CartesianGrid vertical={false} />
          <XAxis
            dataKey="x"
            axisLine={false}
            tickLine={false}
            tick={{ fontSize: 10 }}
            minTickGap={24}
            tickFormatter={(value) => fmt.date(String(value), style)}
          />
          <YAxis
            axisLine={false}
            tickLine={false}
            tick={{ fontSize: 10 }}
            width={58}
            tickFormatter={(value) =>
              hideSensitive ? "" : fiat ? fmt.fiatCompact(Number(value)) : fmt.btcCompact(Number(value))
            }
          />
          <Tooltip
            isAnimationActive={false}
            cursor={{ stroke: "var(--border)", strokeWidth: 1 }}
            content={({ active, payload, label }) =>
              active && payload?.length ? (
                <TooltipFrame
                  sensitive={hideSensitive}
                  title={fmt.date(String(label), style)}
                  rows={payload.map((entry) => {
                    const isBasis = entry.dataKey === "basis";
                    const asset = keys.find((item) => item.key === entry.dataKey)?.asset ?? assets[0] ?? "";
                    const raw = entry.value;
                    return {
                      key: String(entry.dataKey),
                      color: isBasis ? palette.muted : colors.get(asset),
                      label: isBasis ? t("visual.measure.cost_basis") : measure === "compare" ? t("visual.measure.market_value") : asset,
                      value: typeof raw === "number" ? format(raw, asset) : t("visual.unknownValue"),
                    };
                  })}
                />
              ) : null
            }
          />
          {keys.map(({ asset, key }, index) => (
            <Area
              key={key}
              dataKey={key}
              name={asset}
              type="stepAfter"
              stroke={colors.get(asset)}
              strokeWidth={2}
              fill={`url(#fill-${visual.callId}-${key})`}
              connectNulls={false}
              dot={false}
              activeDot={{ r: 4, strokeWidth: 2, stroke: "var(--card)" }}
              isAnimationActive={!reduced}
              animationDuration={1100}
              animationBegin={index * 120}
              animationEasing="ease-out"
            />
          ))}
          {measure === "compare" ? (
            <Line
              dataKey="basis"
              type="stepAfter"
              stroke={palette.muted}
              strokeWidth={1.5}
              dot={false}
              activeDot={{ r: 3, strokeWidth: 2, stroke: "var(--card)" }}
              isAnimationActive={!reduced}
              animationDuration={1100}
              animationBegin={200}
            />
          ) : null}
        </AreaChart>
      </ChartContainer>
      <Legend
        items={[
          ...keys.map(({ asset, key }) => ({
            key,
            label: measure === "compare" ? t("visual.measure.market_value") : asset,
            color: colors.get(asset) ?? palette.muted,
          })),
          ...(measure === "compare" ? [{ key: "basis", label: t("visual.measure.cost_basis"), color: palette.muted }] : []),
        ]}
      />
    </div>
  );
}
