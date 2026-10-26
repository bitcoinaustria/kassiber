import { Ban, Globe2, Laptop } from "lucide-react";

import { cn } from "@/lib/utils";

import type { NetworkReach } from "./networkReach";

const REACH_ICON = { internet: Globe2, device: Laptop, none: Ban } as const;

/** The reach icon and its label, as plain muted text (no pill). */
export const NetworkStatus = ({
  reach,
  label,
  className,
}: {
  reach: NetworkReach;
  label: string;
  className?: string;
}) => {
  const Icon = REACH_ICON[reach];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 text-xs text-ink-3",
        reach === "internet" && "text-ink-2",
        className,
      )}
    >
      <Icon className="size-3.5 shrink-0" aria-hidden="true" />
      {label}
    </span>
  );
};
