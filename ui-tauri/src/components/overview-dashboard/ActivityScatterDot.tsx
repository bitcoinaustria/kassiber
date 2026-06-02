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
  onOpenTransactionDetail,
}: ActivityScatterDotProps) {
  const navigate = useNavigate();
  const openedOnMouseUpRef = React.useRef(false);
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
    if (onOpenTransactionDetail) {
      onOpenTransactionDetail(transactionId);
      return;
    }
    void navigate({ to: transactionDetailHref(transactionId) });
  };
  const handleClick = (event: React.MouseEvent<SVGGElement>) => {
    if (!transactionId) return;
    event.preventDefault();
    event.stopPropagation();
    if (openedOnMouseUpRef.current) {
      openedOnMouseUpRef.current = false;
      return;
    }
    openTransactionDetail();
  };
  const handleMouseUp = (event: React.MouseEvent<SVGGElement>) => {
    if (!transactionId) return;
    event.preventDefault();
    event.stopPropagation();
    openedOnMouseUpRef.current = true;
    openTransactionDetail();
  };
  const handleKeyDown = (event: React.KeyboardEvent<SVGGElement>) => {
    if (!transactionId || (event.key !== "Enter" && event.key !== " ")) return;
    event.preventDefault();
    event.stopPropagation();
    openTransactionDetail();
  };

  const interactiveProps = transactionId
    ? {
        "aria-label": `Open ${activityFlowLabels[payload.eventFlow]} transaction`,
        focusable: true,
        onClick: handleClick,
        onKeyDown: handleKeyDown,
        onMouseUp: handleMouseUp,
        onMouseDown: (event: React.MouseEvent<SVGGElement>) =>
          event.preventDefault(),
        role: "button",
        style: { cursor: "pointer" },
        tabIndex: 0,
      }
    : {
        focusable: false,
        style: { cursor: "default" },
        tabIndex: -1,
      };

  return (
    <g {...interactiveProps}>
      <circle
        cx={cx}
        cy={cy}
        r={Math.max(radius + 6, 10)}
        fill="transparent"
        pointerEvents="all"
      />
      <circle
        className="recharts-scatter-symbol"
        cx={cx}
        cy={cy}
        r={radius}
        fill={activityFlowColors[payload.eventFlow]}
        fillOpacity={
          activeSeries === null || activeSeries === "events" ? 0.92 : 0.28
        }
        pointerEvents="none"
        stroke="var(--background)"
        strokeWidth={2.5}
      />
    </g>
  );
}
