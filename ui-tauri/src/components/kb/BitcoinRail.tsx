import { Fragment } from "react";

import { displayAssetLabel, RAIL_DETAILS, type SwapRail } from "@/lib/bitcoinRail";
import { cn } from "@/lib/utils";

export interface RailIconProps {
  rail: SwapRail;
  size?: "compact" | "regular" | "large";
}

export function RailIcon({ rail, size = "regular" }: RailIconProps) {
  const details = RAIL_DETAILS[rail];
  const frameSize = {
    compact: "size-6",
    regular: "size-7",
    large: "size-12",
  }[size];
  const iconSize = {
    compact: "size-5",
    regular: "size-6",
    large: "size-11",
  }[size];
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center justify-center bg-white shadow-sm ring-1 ring-black/10 dark:ring-white/15",
        size === "large" ? "rounded-md" : "rounded-full",
        frameSize,
      )}
      title={details.label}
    >
      <img
        src={details.icon}
        alt=""
        aria-hidden="true"
        className={cn(
          "object-contain drop-shadow-sm",
          iconSize,
          rail === "liquid" ? "scale-150" : null,
        )}
      />
    </span>
  );
}

export interface RailBadgeProps {
  rail: SwapRail;
  asset: string;
}

export function RailBadge({ rail, asset }: RailBadgeProps) {
  const details = RAIL_DETAILS[rail];
  const railLabel = details.shortLabel.toUpperCase();
  const assetLabel = displayAssetLabel(asset).toUpperCase();
  const labelParts = railLabel === assetLabel ? [assetLabel] : [railLabel, assetLabel];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-2xs font-medium uppercase",
        details.className,
      )}
      title={`${details.label} ${displayAssetLabel(asset)}`}
    >
      {labelParts.map((part, index) => (
        <Fragment key={part}>
          {index > 0 ? <span className="text-current/50">·</span> : null}
          <span>{part}</span>
        </Fragment>
      ))}
    </span>
  );
}
