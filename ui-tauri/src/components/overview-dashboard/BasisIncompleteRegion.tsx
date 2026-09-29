import { useTranslation } from "react-i18next";
import { usePlotArea, useXAxisScale } from "recharts";

import {
  formatTreasuryDetailDate,
  powerLawDaysFor,
  type BasisIncompleteRange,
} from "./model";

/** Annotation uses the existing plot and x scale, including the brush window. */
export function BasisIncompleteRegion({
  range,
  logTime,
}: {
  range: BasisIncompleteRange;
  logTime: boolean;
}) {
  const { t } = useTranslation("overview");
  const plot = usePlotArea();
  const xScale = useXAxisScale();
  if (!plot || !xScale) return null;

  const knownStart = Number.isFinite(range.from);
  const day = knownStart
    ? new Date(range.from).toISOString().slice(0, 10)
    : null;
  const scaledStart = range.coversAll
    ? plot.x
    : xScale(
        logTime
          ? powerLawDaysFor(Date.parse(`${day}T00:00:00Z`))
          : range.firstDate,
      );
  if (scaledStart === undefined || !Number.isFinite(scaledStart)) return null;
  const start = Math.max(plot.x, Math.min(plot.x + plot.width, scaledStart));
  const label = day
    ? t("treasury.basisIncompleteFrom", { date: formatTreasuryDetailDate(day) })
    : t("treasury.basisIncomplete");

  return (
    <g className="basis-incomplete-region" pointerEvents="none">
      <rect
        x={start}
        y={plot.y}
        width={plot.x + plot.width - start}
        height={plot.height}
        fill="color-mix(in oklch, var(--muted) 85%, var(--foreground))"
        fillOpacity={0.3}
      />
      {knownStart && !range.coversAll && (
        <line
          x1={start}
          x2={start}
          y1={plot.y}
          y2={plot.y + plot.height}
          stroke="var(--muted-foreground)"
          strokeOpacity={0.4}
          strokeWidth={1}
          strokeDasharray="3 4"
        />
      )}
      {/* Reserved space above the plot keeps even a late-gap label clear of
          the value tags. HTML wrapping also accommodates German on narrow cards. */}
      <foreignObject x={plot.x} y={plot.y - 40} width={plot.width} height={32}>
        <div className="flex h-full items-end text-2xs leading-tight text-muted-foreground">
          {label}
        </div>
      </foreignObject>
    </g>
  );
}
