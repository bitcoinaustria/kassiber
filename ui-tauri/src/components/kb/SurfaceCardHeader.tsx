import type { ReactNode, Ref } from "react";

import { cn } from "@/lib/utils";

/**
 * The count beside a card's title ("Wallets and sources 13"), as Wallets and
 * Transfers show it.
 */
export const surfaceCountClassName =
  "ml-1 inline-flex items-center rounded-md bg-gray-50 px-2 py-1 text-2xs font-medium text-gray-600 ring-1 ring-gray-500/10 ring-inset sm:text-xs dark:bg-gray-800/50 dark:text-gray-400 dark:ring-gray-400/20";

/**
 * A removable filter chip under a card's header, cleared by a click.
 */
export const surfaceFilterChipClassName =
  "inline-flex items-center gap-1 rounded-md border bg-background px-2 py-1 text-2xs font-medium text-muted-foreground transition-colors hover:bg-muted sm:text-xs";

/**
 * The header row of a titled `kb-surface` card holding a table: the title
 * and its count on the left, filters and actions on the right.
 */
export function SurfaceCardHeader({
  title,
  titleId,
  titleRef,
  count,
  actions,
  className,
  children,
}: {
  title: ReactNode;
  titleId?: string;
  /** For moving focus to the card, e.g. after its last row was decided. */
  titleRef?: Ref<HTMLHeadingElement>;
  count?: ReactNode;
  actions?: ReactNode;
  className?: string;
  /** Extra content after the count, such as active filter chips. */
  children?: ReactNode;
}) {
  return (
    <div
      className={cn(
        "flex flex-col gap-3 p-3 sm:flex-row sm:items-center sm:gap-4 sm:px-6 sm:py-3.5",
        className,
      )}
    >
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
        <h2
          id={titleId}
          ref={titleRef}
          tabIndex={titleRef ? -1 : undefined}
          className="rounded-sm text-sm font-medium focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring sm:text-base"
        >
          {title}
        </h2>
        {count != null ? <span className={surfaceCountClassName}>{count}</span> : null}
        {children}
      </div>
      {actions ? <div className="flex flex-wrap items-center gap-2">{actions}</div> : null}
    </div>
  );
}
