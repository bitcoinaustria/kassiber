import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Bar, BarChart, CartesianGrid, Cell, ReferenceLine, Tooltip, XAxis, YAxis } from "recharts";

import { ChartContainer } from "@/components/ui/chart";
import { usePrefersReducedMotion } from "@/hooks/usePrefersReducedMotion";
import { cn } from "@/lib/utils";
import type { TaxVisual } from "./model";
import type { VisualFormat } from "./format";
import type { VisualPalette } from "./palette";
import { stagger, zeroBasedAxis } from "./motion";
import { GrowBar, Segmented, StatTile, TooltipFrame } from "./primitives";

type Measure = "gain_loss" | "proceeds" | "cost_basis";
const FIELD = { gain_loss: "gainLoss", proceeds: "proceeds", cost_basis: "costBasis" } as const;

interface Props {
  visual: TaxVisual;
  fmt: VisualFormat;
  palette: VisualPalette;
  hideSensitive: boolean;
  expanded?: boolean;
}

export function TaxYearsVisual({ visual, fmt, palette, hideSensitive, expanded }: Props) {
  const { t } = useTranslation("assistant");
  const reduced = usePrefersReducedMotion();
  const [measure, setMeasure] = useState<Measure>((visual.measure as Measure) ?? "gain_loss");
  const field = FIELD[measure];
  const data = visual.years.map((year) => ({ x: String(year.year), year: year.year, value: year[field] }));
  const axis = zeroBasedAxis(data.map((point) => point.value ?? 0));
  const lastActive = [...visual.years].reverse().find((year) => (year.gainLoss ?? 0) !== 0 || (year.proceeds ?? 0) !== 0);
  const [selected, setSelected] = useState<number | null>(lastActive?.year ?? visual.years.at(-1)?.year ?? null);
  const year = visual.years.find((item) => item.year === selected) ?? null;
  const breakdown = useMemo(() => {
    const groups = new Map<string, { key: string; label: string; detail: string; value: number }>();
    for (const row of visual.details) {
      if (row.year !== selected) continue;
      const type = row.capitalGainsType || row.transactionType;
      const key = `${row.asset}\u0000${type}`;
      const entry = groups.get(key) ?? { key, label: row.asset || "—", detail: type, value: 0 };
      entry.value += row[field] ?? 0;
      groups.set(key, entry);
    }
    return [...groups.values()].sort((a, b) => Math.abs(b.value) - Math.abs(a.value));
  }, [visual.details, selected, field]);
  const maxAbs = Math.max(1e-9, ...breakdown.map((item) => Math.abs(item.value)));
  const signedFiat = (value: number) => fmt.signed(fmt.fiat(Math.abs(value)), value);
  const colorFor = (value: number | null) =>
    measure !== "gain_loss" ? palette.series[1] : (value ?? 0) < 0 ? palette.negative : palette.positive;

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="text-2xs text-muted-foreground">{t("visual.tax.pickYear")}</div>
        <Segmented
          label={t("visual.controls.measure")}
          value={measure}
          options={(["gain_loss", "proceeds", "cost_basis"] as const).map((value) => ({ value, label: t(`visual.measure.${value}`) }))}
          onChange={setMeasure}
        />
      </div>
      <ChartContainer config={{}} className={cn("aspect-auto w-full", expanded ? "h-72" : "h-44")}>
        <BarChart data={data} margin={{ top: 8, right: 8, bottom: 0, left: 0 }} onClick={(state) => {
          const label = state?.activeLabel;
          if (label !== undefined) setSelected(Number(label));
        }}>
          <CartesianGrid vertical={false} />
          <XAxis dataKey="x" axisLine={false} tickLine={false} tick={{ fontSize: 10 }} />
          <YAxis axisLine={false} tickLine={false} tick={{ fontSize: 10 }} width={58} domain={axis.domain} ticks={axis.ticks} interval={0} tickFormatter={(value) => (hideSensitive ? "" : fmt.fiatCompact(Number(value)))} />
          <ReferenceLine y={0} stroke="var(--border)" />
          <Tooltip
            isAnimationActive={false}
            cursor={{ fill: "var(--muted)", opacity: 0.4 }}
            content={({ active, payload, label }) => {
              if (!active || !payload?.length) return null;
              const row = visual.years.find((item) => String(item.year) === String(label));
              if (!row) return null;
              return (
                <TooltipFrame
                  sensitive={hideSensitive}
                  title={String(label)}
                  rows={[
                    { key: "g", color: colorFor(row.gainLoss), label: t("visual.measure.gain_loss"), value: row.gainLoss === null ? t("visual.unknownValue") : signedFiat(row.gainLoss) },
                    { key: "p", label: t("visual.measure.proceeds"), value: row.proceeds === null ? t("visual.unknownValue") : fmt.fiat(row.proceeds) },
                    { key: "c", label: t("visual.measure.cost_basis"), value: row.costBasis === null ? t("visual.unknownValue") : fmt.fiat(row.costBasis) },
                  ]}
                />
              );
            }}
          />
          <Bar dataKey="value" maxBarSize={24} radius={[4, 4, 0, 0]} isAnimationActive={!reduced} animationDuration={900} animationEasing="ease-out" className="cursor-pointer">
            {data.map((point) => (
              <Cell
                key={point.x}
                fill={colorFor(point.value)}
                fillOpacity={selected === null || selected === point.year ? 1 : 0.35}
              />
            ))}
          </Bar>
        </BarChart>
      </ChartContainer>
      {year ? (
        <div key={year.year} className="space-y-2">
          <div className="text-xs font-medium">{t("visual.tax.yearDetail", { year: year.year })}</div>
          <div className="grid grid-cols-3 gap-2">
            <StatTile label={t("visual.measure.proceeds")} value={year.proceeds} format={fmt.fiat} sensitive={hideSensitive} />
            <StatTile label={t("visual.measure.cost_basis")} value={year.costBasis} format={fmt.fiat} sensitive={hideSensitive} delayMs={70} />
            <StatTile label={t("visual.measure.gain_loss")} value={year.gainLoss} format={signedFiat} sensitive={hideSensitive} delayMs={140} />
          </div>
          {breakdown.length ? (
            <ul className="space-y-1">
              {breakdown.map((item, index) => (
                <li key={item.key} className="kb-visual-rise grid grid-cols-[minmax(0,10rem)_minmax(0,1fr)_auto] items-center gap-2 text-xs" style={{ animationDelay: `${stagger(index)}ms` }}>
                  <span className="truncate">
                    <span className="font-medium">{item.label}</span>
                    {item.detail ? <span className="text-muted-foreground"> · {t(`visual.tax.type.${item.detail}`, { defaultValue: item.detail })}</span> : null}
                  </span>
                  <span className="h-1.5 rounded-full bg-muted/50">
                    <GrowBar fraction={Math.abs(item.value) / maxAbs} color={colorFor(item.value)} delayMs={stagger(index) + 60} />
                  </span>
                  <span className={cn("text-right tabular-nums", hideSensitive && "sensitive")}>
                    {measure === "gain_loss" ? signedFiat(item.value) : fmt.fiat(item.value)}
                  </span>
                </li>
              ))}
            </ul>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
