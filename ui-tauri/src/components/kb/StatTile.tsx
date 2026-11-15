import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";

import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";

/**
 * One tile of a screen's stat strip: an icon and label, the big value and a
 * sub-line. Transactions and Transfers lay their tiles out in their own grid
 * (`kb-surface grid …`) and draw each tile with this, so the strips read the
 * same. A tile with `onClick` is a button that fills from the left on hover.
 */
export interface StatTileProps {
  label: string;
  icon: LucideIcon;
  /** Text colour of the icon, e.g. `text-emerald-600`. */
  tone?: string;
  value: ReactNode;
  meta?: ReactNode;
  /** No figure yet: placeholders, not clickable. */
  loading?: boolean;
  /** The figure on screen is being refreshed: dimmed, still clickable. */
  refreshing?: boolean;
  onClick?: () => void;
  ariaLabel?: string;
  /** Marks a tile that filters the list below while its filter is on. */
  pressed?: boolean;
  /** Borders for the tile's place in its grid. */
  className?: string;
}

export const statTileBaseClassName = "min-w-0 space-y-2 p-3 text-left @2xl/page:p-4";

export const statTileButtonClassName =
  "relative isolate w-full cursor-pointer overflow-hidden transition-colors before:absolute before:inset-0 before:z-0 before:origin-left before:scale-x-0 before:bg-muted/60 before:content-[''] before:transition-transform before:duration-200 before:ease-out hover:before:scale-x-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:before:scale-x-100 [&>*]:relative [&>*]:z-10";

export function StatTile({
  label,
  icon: Icon,
  tone,
  value,
  meta,
  loading = false,
  refreshing = false,
  onClick,
  ariaLabel,
  pressed,
  className,
}: StatTileProps) {
  const interactive = Boolean(onClick) && !loading;
  const tileClassName = cn(
    statTileBaseClassName,
    interactive && statTileButtonClassName,
    pressed && "bg-muted/40",
    className,
  );
  const content = loading ? (
    <>
      <div className="flex items-center gap-2">
        <Skeleton className="size-4 rounded-md" />
        <Skeleton className="h-3 w-20" />
      </div>
      <Skeleton className="h-6 w-24" />
      <Skeleton className="h-3 w-16" />
    </>
  ) : (
    <>
      <div className="flex items-center gap-2 text-xs text-muted-foreground">
        <Icon className={cn("size-4", tone)} aria-hidden="true" />
        <span className="truncate">{label}</span>
      </div>
      <div
        className={cn(
          "truncate text-xl font-semibold tracking-tight transition-opacity motion-reduce:transition-none",
          refreshing && "opacity-60",
        )}
      >
        {value}
      </div>
      {meta != null ? (
        <div className="truncate text-xs text-muted-foreground">{meta}</div>
      ) : null}
    </>
  );
  return interactive ? (
    <button
      type="button"
      className={tileClassName}
      onClick={onClick}
      aria-label={ariaLabel}
      aria-pressed={pressed}
    >
      {content}
    </button>
  ) : (
    <div className={tileClassName}>{content}</div>
  );
}
