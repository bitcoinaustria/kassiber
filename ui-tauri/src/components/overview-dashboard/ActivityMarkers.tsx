import { useNavigate } from "@tanstack/react-router";
import * as React from "react";

import {
  ACTIVITY_MARKER_SLIDER_MARKS,
  activityFlowColors,
  activityFlowLabels,
  activityMarkerSliderValue,
  serializeActivityMarkerMinimum,
  transactionDetailHref,
  type TreasuryChartPoint,
  type TreasuryChartSeriesKey,
} from "./shared";

export type ActivityScatterDotProps = {
  cx?: number;
  cy?: number;
  size?: number;
  payload?: TreasuryChartPoint;
  activeSeries: TreasuryChartSeriesKey | null;
};

export function ActivityMarkerSlider({
  id,
  label,
  value,
  color,
  onChange,
}: {
  id: string;
  label: string;
  value: number;
  color: string;
  onChange: (value: number) => void;
}) {
  const marksId = `${id}-marks`;
  return (
    <div className="mt-3 space-y-2">
      <input
        aria-label={label}
        className="h-2 w-full cursor-pointer"
        list={marksId}
        min={0}
        max={ACTIVITY_MARKER_SLIDER_MARKS.length - 1}
        step={1}
        type="range"
        value={activityMarkerSliderValue(value)}
        style={{ accentColor: color }}
        onChange={(event) =>
          onChange(ACTIVITY_MARKER_SLIDER_MARKS[Number(event.currentTarget.value)] ?? 0)
        }
      />
      <datalist id={marksId}>
        {ACTIVITY_MARKER_SLIDER_MARKS.map((mark, index) => (
          <option key={mark} value={index} label={serializeActivityMarkerMinimum(mark)} />
        ))}
      </datalist>
      <div className="flex justify-between text-[10px] text-muted-foreground">
        {ACTIVITY_MARKER_SLIDER_MARKS.map((mark) => (
          <span key={mark} className="tabular-nums">
            {serializeActivityMarkerMinimum(mark)}
          </span>
        ))}
      </div>
    </div>
  );
}

export function ActivityScatterDot({
  cx,
  cy,
  size,
  payload,
  activeSeries,
}: ActivityScatterDotProps) {
  const navigate = useNavigate();
  if (
    typeof cx !== "number" ||
    typeof cy !== "number" ||
    !payload?.eventFlow
  ) {
    return null;
  }

  const normalizedSize = typeof size === "number" ? size : 80;
  const radius = Math.max(3, Math.sqrt(normalizedSize / Math.PI));
  const transactionId = payload.eventTransactionId ?? payload.eventId;
  const openTransactionDetail = () => {
    if (!transactionId) return;
    void navigate({ to: transactionDetailHref(transactionId) });
  };
  const handleClick = (event: React.MouseEvent<SVGCircleElement>) => {
    if (!transactionId) return;
    event.preventDefault();
    event.stopPropagation();
    openTransactionDetail();
  };
  const handleKeyDown = (event: React.KeyboardEvent<SVGCircleElement>) => {
    if (!transactionId || (event.key !== "Enter" && event.key !== " ")) return;
    event.preventDefault();
    event.stopPropagation();
    openTransactionDetail();
  };

  return (
    <circle
      className="recharts-scatter-symbol"
      cx={cx}
      cy={cy}
      r={radius}
      aria-label={
        transactionId
          ? `Open ${activityFlowLabels[payload.eventFlow]} transaction`
          : undefined
      }
      fill={activityFlowColors[payload.eventFlow]}
      fillOpacity={
        activeSeries === null || activeSeries === "events" ? 0.92 : 0.28
      }
      focusable={transactionId ? true : false}
      onClick={handleClick}
      onKeyDown={handleKeyDown}
      onMouseDown={(event) => event.preventDefault()}
      role={transactionId ? "button" : undefined}
      stroke="var(--background)"
      strokeWidth={2.5}
      style={{
        cursor: transactionId ? "pointer" : "default",
      }}
      tabIndex={transactionId ? 0 : -1}
    />
  );
}
