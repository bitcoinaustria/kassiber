import * as React from "react";
import { cn } from "@/lib/utils";

interface KbCardProps {
  title?: React.ReactNode;
  action?: React.ReactNode;
  className?: string;
  pad?: boolean;
  children: React.ReactNode;
}

/**
 * Kassiber Card — hard-edge, tokenized background, optional title bar.
 *
 * Visually closer to claude-design's `Card` than shadcn's default
 * (rounded-xl + shadow-sm + py-6). When the standard shadcn look is
 * needed for some future surface, import directly from
 * `components/ui/card`.
 */
export function KbCard({
  title,
  action,
  className,
  pad = true,
  children,
}: KbCardProps) {
  return (
    <div
      className={cn(
        "flex min-h-0 flex-col border border-border bg-card text-card-foreground",
        className,
      )}
      data-slot="kb-card"
    >
      {title !== undefined && (
        <div className="flex h-9 flex-shrink-0 items-center justify-between border-b border-border px-3.5">
          <span className="font-sans text-[13px] font-semibold tracking-[0.005em] text-foreground">
            {title}
          </span>
          {action}
        </div>
      )}
      <div
        className={cn(
          "flex-1 min-h-0 overflow-auto",
          pad && "p-3.5",
        )}
      >
        {children}
      </div>
    </div>
  );
}
