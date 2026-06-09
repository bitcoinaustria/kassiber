import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/**
 * Small inline count chip used next to card titles ("UTXOs 12",
 * "Recent transactions 6"). Uses semantic theme tokens so it tracks the
 * active theme instead of hardcoded gray scales.
 */
export function CountBadge({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-md bg-muted px-2 py-1 text-[10px] font-medium text-muted-foreground ring-1 ring-border/60 ring-inset sm:text-xs",
        className,
      )}
    >
      {children}
    </span>
  );
}
