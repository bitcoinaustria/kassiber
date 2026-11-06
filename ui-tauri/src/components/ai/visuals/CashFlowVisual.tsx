import { useState } from "react";
import { useTranslation } from "react-i18next";
import { ArrowRightLeft, Repeat } from "lucide-react";
import { Bar, BarChart, CartesianGrid, ReferenceLine, Tooltip, XAxis, YAxis } from "recharts";

import { ChartContainer } from "@/components/ui/chart";
import { usePrefersReducedMotion } from "@/hooks/usePrefersReducedMotion";
import { cn } from "@/lib/utils";
import type { CashFlowVisual as CashFlowVisualModel } from "./model";
import type { VisualFormat } from "./format";
import type { VisualPalette } from "./palette";
import { zeroBasedAxis } from "./motion";
import { Legend, Segmented, StatTile, TooltipFrame } from "./primitives";

type Unit = "quantity" | "fiat_value";

interface Props {
  visual: CashFlowVisualModel;
  fmt: VisualFormat;
  palette: VisualPalette;
  hideSensitive: boolean;
  expanded?: boolean;
}

export function CashFlowVisual({ visual, fmt, palette, hideSensitive, expanded }: Props) {
  const { t } = useTranslation("assistant");
  const reduced = usePrefersReducedMotion();
  const [unit, setUnit] = useState<Unit>(visual.measure === "fiat_value" ? "fiat_value" : "quantity");
  const fiat = unit === "fiat_value";
  const format = (value: number) => (fiat ? fmt.fiat(value) : fmt.btc(value));
  const compact = (value: number) => (fiat ? fmt.fiatCompact(value) : fmt.btcCompact(value));
  const { incoming, outgoing, transfers, swaps } = visual.totals;
  const net = fiat ? incoming.fiat - outgoing.fiat : incoming.btc - outgoing.btc;
  // Sent sits below the baseline: one axis, signed, never a second scale.
  const data = visual.rows.map((row) => ({
    x: row.period,
    in: fiat ? row.inFiat : row.inBtc,
    out: -(fiat ? row.outFiat : row.outBtc),
    row,
  }));
  const style = visual.bucket;
  const axis = zeroBasedAxis(data.flatMap((point) => [point.in, point.out]));

  return (
    <div className="space-y-3">
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
        <StatTile
          label={t("visual.cashFlow.received")}
          value={fiat ? incoming.fiat : incoming.btc}
          format={format}
          hint={t("visual.cashFlow.count", { count: incoming.count })}
          sensitive={hideSensitive}
        />
        <StatTile
          label={t("visual.cashFlow.sent")}
          value={fiat ? outgoing.fiat : outgoing.btc}
          format={format}
          hint={t("visual.cashFlow.count", { count: outgoing.count })}
          sensitive={hideSensitive}
          delayMs={70}
        />
        <StatTile
          label={t("visual.cashFlow.net")}
          value={net}
          format={(value) => fmt.signed(format(Math.abs(value)), value)}
          sensitive={hideSensitive}
          delayMs={140}
        />
        <StatTile
          label={t("visual.cashFlow.largest")}
          value={incoming.largest ? (fiat ? incoming.largest.fiat : incoming.largest.btc) : null}
          format={format}
          hint={incoming.largest?.label || undefined}
          sensitive={hideSensitive}
          delayMs={210}
        />
      </div>
      <div className="flex flex-wrap items-center justify-between gap-2">
        <Legend
          shape="square"
          items={[
            { key: "in", label: t("visual.cashFlow.received"), color: palette.positive },
            { key: "out", label: t("visual.cashFlow.sent"), color: palette.negative },
          ]}
        />
        <Segmented
          label={t("visual.controls.unit")}
          value={unit}
          options={[
            { value: "quantity" as const, label: "₿" },
            { value: "fiat_value" as const, label: visual.fiatCurrency },
          ]}
          onChange={setUnit}
        />
      </div>
      <ChartContainer config={{}} className={cn("aspect-auto w-full", expanded ? "h-80" : "h-48")}>
        <BarChart data={data} stackOffset="sign" margin={{ top: 8, right: 8, bottom: 0, left: 0 }}>
          <CartesianGrid vertical={false} />
          <XAxis dataKey="x" axisLine={false} tickLine={false} tick={{ fontSize: 10 }} minTickGap={16} tickFormatter={(value) => fmt.date(String(value), style)} />
          <YAxis axisLine={false} tickLine={false} tick={{ fontSize: 10 }} width={58} domain={axis.domain} ticks={axis.ticks} interval={0} tickFormatter={(value) => (hideSensitive ? "" : fmt.signed(compact(Math.abs(Number(value))), Number(value) < 0 ? -1 : 0))} />
          <ReferenceLine y={0} stroke="var(--border)" />
          <Tooltip
            isAnimationActive={false}
            cursor={{ fill: "var(--muted)", opacity: 0.4 }}
            content={({ active, payload, label }) => {
              const point = payload?.[0]?.payload as (typeof data)[number] | undefined;
              if (!active || !point) return null;
              const { row } = point;
              return (
                <TooltipFrame
                  sensitive={hideSensitive}
                  title={fmt.date(String(label), style)}
                  rows={[
                    { key: "in", color: palette.positive, label: t("visual.cashFlow.receivedCount", { count: row.inCount }), value: format(point.in) },
                    { key: "out", color: palette.negative, label: t("visual.cashFlow.sentCount", { count: row.outCount }), value: format(-point.out) },
                    { key: "net", label: t("visual.cashFlow.net"), value: fmt.signed(format(Math.abs(point.in + point.out)), point.in + point.out) },
                    ...(row.transferCount ? [{ key: "tr", label: t("visual.cashFlow.transfers", { count: row.transferCount }), value: String(row.transferCount) }] : []),
                    ...(row.missingPrices && fiat ? [{ key: "mp", label: t("visual.cashFlow.unpriced", { count: row.missingPrices }), value: "!" }] : []),
                  ]}
                />
              );
            }}
          />
          <Bar dataKey="in" stackId="flow" fill={palette.positive} maxBarSize={24} radius={[4, 4, 0, 0]} isAnimationActive={!reduced} animationDuration={900} animationEasing="ease-out" />
          <Bar dataKey="out" stackId="flow" fill={palette.negative} maxBarSize={24} radius={[4, 4, 0, 0]} isAnimationActive={!reduced} animationDuration={900} animationBegin={120} animationEasing="ease-out" />
        </BarChart>
      </ChartContainer>
      {transfers.count || swaps.count ? (
        <div className="flex flex-wrap gap-2 text-2xs text-muted-foreground">
          {transfers.count ? (
            <span className="inline-flex items-center gap-1 rounded-full bg-muted/60 px-2 py-0.5">
              <ArrowRightLeft className="size-3" aria-hidden="true" />
              {t("visual.cashFlow.transfersExcluded", { count: transfers.count })}
            </span>
          ) : null}
          {swaps.count ? (
            <span className="inline-flex items-center gap-1 rounded-full bg-muted/60 px-2 py-0.5">
              <Repeat className="size-3" aria-hidden="true" />
              {t("visual.cashFlow.swapsExcluded", { count: swaps.count })}
            </span>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
