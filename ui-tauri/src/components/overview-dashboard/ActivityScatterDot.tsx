import { useNavigate } from "@tanstack/react-router";
import * as React from "react";
import { useTranslation } from "react-i18next";

import {
  activityFlowLabelKeys,
  activityFlowShares,
  activityFlowSlicePath,
  transactionDetailHref,
  transactionSetHref,
  type ActivityScatterDotProps,
} from "./model";

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
  const { t } = useTranslation("overview");
  const openedOnPointerDownRef = React.useRef(false);
  if (typeof cx !== "number" || typeof cy !== "number" || !payload?.eventFlow) {
    return null;
  }

  const parentFlow = payload.eventFlow;
  const normalizedSize = typeof size === "number" ? size : 80;
  const radius = Math.max(3, Math.sqrt(normalizedSize / Math.PI));
  const markerCount = payload.markerCount ?? 1;
  const groupedPoints = payload.markerGroupedPoints ?? [];
  const transactionId =
    markerCount > 1 ? undefined : (payload.eventTransactionId ?? payload.eventId);
  // Every id in the bucket: the dot's count badge promises this many, so a cap
  // here would silently open fewer than it advertises. In-app navigation, so
  // the href length is a router concern, not an HTTP one.
  const groupedTransactionIds = [
    ...new Set(
      groupedPoints
        .map((point) => point.eventTransactionId ?? point.eventId)
        .filter((id): id is string => Boolean(id)),
    ),
  ];
  const canOpenMarker = Boolean(transactionId || groupedTransactionIds.length > 0);
  const slices = payload.markerMixedFlows ? activityFlowShares(groupedPoints) : [];
  const dimmed = activeSeries !== null && activeSeries !== "events";
  const fillOpacity = dimmed ? 0.28 : 0.92;

  const openMarker = () => {
    if (transactionId) {
      if (onOpenTransactionDetail) {
        onOpenTransactionDetail(transactionId);
        return;
      }
      void navigate({ to: transactionDetailHref(transactionId) });
      return;
    }
    if (groupedTransactionIds.length === 0) return;
    void navigate({ to: transactionSetHref(groupedTransactionIds) });
  };
  const handlePointerDown = (event: React.PointerEvent<SVGGElement>) => {
    if (!canOpenMarker) return;
    event.preventDefault();
    event.stopPropagation();
    openedOnPointerDownRef.current = true;
    openMarker();
  };
  const handleClick = (event: React.MouseEvent<SVGGElement>) => {
    if (!canOpenMarker) return;
    event.preventDefault();
    event.stopPropagation();
    if (openedOnPointerDownRef.current) {
      openedOnPointerDownRef.current = false;
      return;
    }
    openMarker();
  };
  const handleKeyDown = (event: React.KeyboardEvent<SVGGElement>) => {
    if (!canOpenMarker || (event.key !== "Enter" && event.key !== " ")) return;
    event.preventDefault();
    event.stopPropagation();
    openMarker();
  };
  const handleEnter = () => onHoverActivityPoint?.(payload);
  const handleLeave = () => onHoverActivityPoint?.(null);

  let sliceStart = 0;

  return (
    <g
      aria-label={
        !canOpenMarker
          ? undefined
          : markerCount > 1
            ? t("treasury.marker.openGroup", { count: markerCount })
            : t("treasury.marker.openTransaction", {
                flow: t(activityFlowLabelKeys[parentFlow]),
              })
      }
      className="group/activity-marker outline-none"
      data-activity-marker="true"
      focusable={canOpenMarker}
      onBlur={handleLeave}
      onClick={handleClick}
      onFocus={handleEnter}
      onKeyDown={handleKeyDown}
      onMouseDown={(event: React.MouseEvent<SVGGElement>) =>
        event.preventDefault()
      }
      onMouseEnter={handleEnter}
      onMouseLeave={handleLeave}
      onPointerDown={handlePointerDown}
      onPointerEnter={handleEnter}
      onPointerLeave={handleLeave}
      role={canOpenMarker ? "button" : undefined}
      style={{ cursor: canOpenMarker ? "pointer" : "default" }}
      tabIndex={canOpenMarker ? 0 : -1}
    >
      <circle
        cx={cx}
        cy={cy}
        // Merged dots draw larger and sit a bucket-gap apart, so they get a
        // tighter pad — an over-wide hit circle would swallow its neighbour's
        // clicks at the tightest spacing.
        r={markerCount > 1 ? Math.max(radius + 2, 8) : Math.max(radius + 6, 10)}
        fill="transparent"
        pointerEvents="all"
      />
      <g
        className="recharts-scatter-symbol transition-transform duration-150 ease-out group-hover/activity-marker:scale-110 group-focus/activity-marker:scale-110"
        pointerEvents="none"
        // User-space origin, not `transform-box: fill-box` — WKWebView is
        // unreliable about fill-box on a <g>.
        style={{ transformOrigin: `${cx}px ${cy}px` }}
      >
        <circle
          cx={cx}
          cy={cy}
          r={radius}
          fill={flowColors[parentFlow]}
          fillOpacity={fillOpacity}
          stroke="var(--background)"
          strokeWidth={2.5}
        />
        {/* A mixed bucket reads as one dot split by flow, so "3 events, mostly
            out" is legible without hovering. */}
        {slices.length > 1 &&
          slices.map(({ flow, share }) => {
            const from = sliceStart;
            sliceStart += share;
            return (
              <path
                key={flow}
                d={activityFlowSlicePath(cx, cy, radius, from, sliceStart)}
                fill={flowColors[flow]}
                fillOpacity={fillOpacity}
              />
            );
          })}
        {markerCount > 1 && (
          <circle
            cx={cx}
            cy={cy}
            r={radius}
            fill="none"
            stroke="var(--background)"
            strokeWidth={2.5}
          />
        )}
      </g>
      {markerCount > 1 && (
        <text
          x={cx}
          y={cy - radius - 4}
          textAnchor="middle"
          className="fill-foreground text-2xs font-semibold tabular-nums"
          // Halo the digits so they stay readable over the balance line.
          paintOrder="stroke"
          stroke="var(--background)"
          strokeWidth={3}
          strokeLinejoin="round"
          pointerEvents="none"
        >
          {markerCount}
        </text>
      )}
    </g>
  );
}
