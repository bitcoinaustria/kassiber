import { useNavigate } from "@tanstack/react-router";
import * as React from "react";

import {
  activityFlowColors,
  activityFlowLabels,
  transactionDetailHref,
  type ActivityScatterDotProps,
} from "./model";

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
