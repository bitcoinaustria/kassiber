import { useNavigate } from "@tanstack/react-router";
import * as React from "react";

import {
  activityFlowLabels,
  transactionDetailHref,
  transactionSetHref,
  type ActivityFlow,
  type ActivityScatterDotProps,
  type TreasuryChartPoint,
} from "./model";

type MarkerEvent =
  | React.MouseEvent<SVGGElement>
  | React.PointerEvent<SVGGElement>
  | React.KeyboardEvent<SVGGElement>;

export function ActivityScatterDot({
  cx,
  cy,
  size,
  payload,
  activeSeries,
  flowColors,
  onOpenTransactionDetail,
  onHoverActivityPoint,
}: ActivityScatterDotProps) {
  const navigate = useNavigate();
  const openedOnPointerDownRef = React.useRef(false);
  if (
    typeof cx !== "number" ||
    typeof cy !== "number" ||
    !payload?.eventFlow
  ) {
    return null;
  }

  const normalizedSize = typeof size === "number" ? size : 80;
  const radius = Math.max(3, Math.sqrt(normalizedSize / Math.PI));
  const markerCount = payload.markerCount ?? 1;
  const allGroupedPoints = payload.markerGroupedPoints ?? [];
  const groupedPoints = allGroupedPoints.slice(0, 9);
  const groupedTransactionIds = [
    ...new Set(
      allGroupedPoints
        .map((point) => point.eventTransactionId ?? point.eventId)
        .filter((id): id is string => Boolean(id)),
    ),
  ];
  const parentFill = payload.markerMixedFlows
    ? "var(--muted-foreground)"
    : flowColors[payload.eventFlow];
  const transactionId =
    markerCount > 1 ? undefined : (payload.eventTransactionId ?? payload.eventId);
  const canOpenMarker = Boolean(transactionId || groupedTransactionIds.length > 0);
  const openTransactionDetailById = (id: string | undefined) => {
    if (!id) return;
    if (onOpenTransactionDetail) {
      onOpenTransactionDetail(id);
      return;
    }
    void navigate({ to: transactionDetailHref(id) });
  };
  const openTransactionDetail = () => {
    if (!transactionId) return;
    openTransactionDetailById(transactionId);
  };
  const openTransactionSet = () => {
    if (groupedTransactionIds.length === 0) return;
    void navigate({ to: transactionSetHref(groupedTransactionIds) });
  };
  const openCurrentMarker = () => {
    if (transactionId) {
      openTransactionDetail();
    } else {
      openTransactionSet();
    }
  };
  const openGroupedPoint = (point: TreasuryChartPoint, event: MarkerEvent) => {
    const groupedTransactionId = point.eventTransactionId ?? point.eventId;
    if (!groupedTransactionId) return;
    event.preventDefault();
    event.stopPropagation();
    openTransactionDetailById(groupedTransactionId);
  };
  const handlePointerDown = (event: React.PointerEvent<SVGGElement>) => {
    if (!canOpenMarker) return;
    event.preventDefault();
    event.stopPropagation();
    openedOnPointerDownRef.current = true;
    openCurrentMarker();
  };
  const handleClick = (event: React.MouseEvent<SVGGElement>) => {
    if (!canOpenMarker) return;
    event.preventDefault();
    event.stopPropagation();
    if (openedOnPointerDownRef.current) {
      openedOnPointerDownRef.current = false;
      return;
    }
    openCurrentMarker();
  };
  const handleKeyDown = (event: React.KeyboardEvent<SVGGElement>) => {
    if (!canOpenMarker || (event.key !== "Enter" && event.key !== " ")) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    openCurrentMarker();
  };
  const handlePointerEnter = () => {
    onHoverActivityPoint?.(payload);
  };
  const handlePointerLeave = () => {
    onHoverActivityPoint?.(null);
  };

  const interactiveProps = transactionId
    ? {
        "aria-label": `Open ${activityFlowLabels[payload.eventFlow]} transaction`,
        className: "group/activity-marker outline-none",
        "data-activity-marker": true,
        focusable: true,
        onClick: handleClick,
        onBlur: handlePointerLeave,
        onFocus: handlePointerEnter,
        onKeyDown: handleKeyDown,
        onMouseEnter: handlePointerEnter,
        onMouseLeave: handlePointerLeave,
        onPointerDown: handlePointerDown,
        onPointerEnter: handlePointerEnter,
        onPointerLeave: handlePointerLeave,
        onMouseDown: (event: React.MouseEvent<SVGGElement>) =>
          event.preventDefault(),
        role: "button",
        style: { cursor: "pointer" },
        tabIndex: 0,
      }
    : {
        "aria-label":
          markerCount > 1
            ? `Open ${markerCount} grouped transactions`
            : undefined,
        className: "group/activity-marker outline-none",
        "data-activity-marker": true,
        focusable: canOpenMarker,
        onClick: handleClick,
        onBlur: handlePointerLeave,
        onFocus: handlePointerEnter,
        onKeyDown: handleKeyDown,
        onMouseEnter: handlePointerEnter,
        onMouseLeave: handlePointerLeave,
        onPointerDown: handlePointerDown,
        onPointerEnter: handlePointerEnter,
        onPointerLeave: handlePointerLeave,
        role: canOpenMarker ? "button" : undefined,
        style: { cursor: canOpenMarker ? "pointer" : "default" },
        tabIndex: canOpenMarker ? 0 : -1,
      };
  const childRadius = Math.max(3.5, Math.min(radius * 0.95, 5.5));
  const spreadRadius = Math.max(radius + 12, 18);
  const flowForPoint = (point: TreasuryChartPoint): ActivityFlow =>
    point.eventFlow ?? payload.eventFlow;
  const flowIndex = new Map<ActivityFlow, number>();
  const flowTotal = groupedPoints.reduce<Record<ActivityFlow, number>>(
    (totals, point) => {
      const flow = flowForPoint(point);
      totals[flow] = (totals[flow] ?? 0) + 1;
      return totals;
    },
    { incoming: 0, outgoing: 0, movement: 0, fee: 0 },
  );
  const childPoints = groupedPoints.map((point) => {
    const flow = flowForPoint(point);
    const index = flowIndex.get(flow) ?? 0;
    flowIndex.set(flow, index + 1);
    const total = flowTotal[flow] ?? 1;
    const offset = (index - (total - 1) / 2) * Math.max(childRadius * 2.8, 7);
    const axis =
      flow === "incoming"
        ? { x: offset, y: -spreadRadius }
        : flow === "outgoing" || flow === "fee"
          ? { x: offset, y: spreadRadius }
          : { x: (index % 2 === 0 ? -1 : 1) * spreadRadius, y: offset };
    return {
      point,
      x: cx + axis.x,
      y: cy + axis.y,
    };
  });

  return (
    <g {...interactiveProps}>
      {markerCount > 1 && (
        <circle
          cx={cx}
          cy={cy}
          r={spreadRadius + childRadius + 7}
          fill="transparent"
          pointerEvents="all"
        />
      )}
      <circle
        cx={cx}
        cy={cy}
        r={markerCount > 1 ? Math.max(radius + 2, 8) : Math.max(radius + 6, 10)}
        fill="transparent"
        pointerEvents="all"
      />
      <circle
        className="recharts-scatter-symbol transition-transform duration-150 ease-out group-hover/activity-marker:scale-110 group-focus/activity-marker:scale-110"
        cx={cx}
        cy={cy}
        r={radius}
        fill={parentFill}
        fillOpacity={
          payload.markerMixedFlows
            ? activeSeries === null || activeSeries === "events"
              ? 0.82
              : 0.25
            : activeSeries === null || activeSeries === "events"
              ? 0.92
              : 0.28
        }
        pointerEvents="none"
        stroke="var(--background)"
        strokeWidth={2.5}
        style={{ transformBox: "fill-box", transformOrigin: "center" }}
      />
      {markerCount > 1 && childPoints.length > 1 && (
        <g
          className="opacity-[0.02] transition-opacity duration-150 ease-out group-hover/activity-marker:opacity-100 group-focus/activity-marker:opacity-100"
        >
          {childPoints.map(({ point, x, y }, index) => (
            <g
              key={`${point.eventTransactionId ?? point.eventId ?? index}`}
              aria-label={`Open ${activityFlowLabels[flowForPoint(point)]} transaction`}
              data-activity-marker="true"
              focusable
              role="button"
              tabIndex={0}
              style={{ cursor: "pointer" }}
              onClick={(event) => openGroupedPoint(point, event)}
              onKeyDown={(event) => {
                if (event.key !== "Enter" && event.key !== " ") return;
                openGroupedPoint(point, event);
              }}
              onPointerDown={(event) => openGroupedPoint(point, event)}
            >
              <line
                x1={cx}
                y1={cy}
                x2={x}
                y2={y}
                stroke={flowColors[flowForPoint(point)]}
                strokeOpacity={0.32}
                strokeWidth={1}
                pointerEvents="none"
              />
              <circle
                cx={x}
                cy={y}
                r={childRadius}
                fill={flowColors[flowForPoint(point)]}
                fillOpacity={0.96}
                stroke="var(--background)"
                strokeWidth={1.5}
                pointerEvents="all"
              />
            </g>
          ))}
        </g>
      )}
      {markerCount > 1 && (
        <text
          x={cx + radius + 4}
          y={cy - radius - 2}
          className="fill-foreground text-[10px] font-semibold tabular-nums transition-opacity duration-150 group-hover/activity-marker:opacity-45 group-focus/activity-marker:opacity-45"
          pointerEvents="none"
        >
          {markerCount}
        </text>
      )}
    </g>
  );
}
