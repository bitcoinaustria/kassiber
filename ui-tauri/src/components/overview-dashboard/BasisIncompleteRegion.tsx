import { AlertTriangle } from "lucide-react";
import * as React from "react";
import { useTranslation } from "react-i18next";
import { usePlotArea, useXAxisScale } from "recharts";

import type { FiatCompleteness } from "@/mocks/seed";

import { BasisGapPopover } from "./BasisGapPopover";
import {
  formatTreasuryDetailDate,
  powerLawDaysFor,
  type BasisIncompleteRange,
} from "./model";

/**
 * Hatches the part of the chart whose cost basis is incomplete and labels its
 * start where it begins. The label opens what causes it and where to fix it.
 * Uses the existing plot and x scale, including the brush window.
 */
export function BasisIncompleteRegion({
  range,
  logTime,
  completeness,
}: {
  range: BasisIncompleteRange;
  logTime: boolean;
  completeness: FiatCompleteness;
}) {
  const { t } = useTranslation("overview");
  const hatchId = `basis-gap-hatch-${React.useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
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
  // The label hangs off the line into the hatched side, unless that side is
  // too narrow; then it sits before the line.
  const anchorRight = start - plot.x > plot.width / 2;
  const stopChartGesture = (event: React.SyntheticEvent) =>
    event.stopPropagation();

  return (
    <g className="basis-incomplete-region" pointerEvents="none">
      <defs>
        <pattern
          id={hatchId}
          width={6}
          height={6}
          patternUnits="userSpaceOnUse"
          patternTransform="rotate(45)"
        >
          <line
            x1={0}
            y1={0}
            x2={0}
            y2={6}
            stroke="var(--muted-foreground)"
            strokeOpacity={0.14}
            strokeWidth={1}
          />
        </pattern>
      </defs>
      <rect
        x={start}
        y={plot.y}
        width={plot.x + plot.width - start}
        height={plot.height}
        fill={`url(#${hatchId})`}
      />
      {knownStart && !range.coversAll && (
        <line
          x1={start}
          x2={start}
          y1={plot.y - 8}
          y2={plot.y + plot.height}
          stroke="var(--color-amber-500, #f59e0b)"
          strokeOpacity={0.6}
          strokeWidth={1}
          strokeDasharray="3 4"
        />
      )}
      {/* Reserved space above the plot keeps the label clear of the value
          tags. HTML lets the label be a real button. */}
      <foreignObject x={plot.x} y={plot.y - 40} width={plot.width} height={32}>
        <div className="relative h-full">
          <BasisGapPopover
            completeness={completeness}
            align={anchorRight ? "end" : "start"}
          >
            <button
              type="button"
              data-basis-gap-label="true"
              className="pointer-events-auto absolute bottom-0 inline-flex max-w-full items-center gap-1 rounded-full border border-amber-500/40 bg-amber-500/10 px-2 py-0.5 text-2xs font-medium leading-4 text-amber-700 hover:bg-amber-500/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring dark:text-amber-300"
              style={
                anchorRight
                  ? { right: plot.x + plot.width - start }
                  : { left: start - plot.x }
              }
              onMouseDown={stopChartGesture}
              onClick={stopChartGesture}
            >
              <AlertTriangle className="size-3 shrink-0" aria-hidden="true" />
              <span className="truncate">{label}</span>
            </button>
          </BasisGapPopover>
        </div>
      </foreignObject>
    </g>
  );
}
