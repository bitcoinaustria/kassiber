import { Link } from "@tanstack/react-router";
import type * as React from "react";

import { cn } from "@/lib/utils";

/**
 * The book's readiness for reports, shown first (left) in a page header on
 * Overview and Reports alike: the same shape, colours, and wording, so the
 * two pages never look like they disagree. What it says comes from each
 * page's readiness check; this only draws it.
 */
export type ReadinessTone = "good" | "warning" | "alert" | "neutral";

const readinessToneStyles: Record<ReadinessTone, string> = {
  good: "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  warning: "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  alert: "border-red-500/25 bg-red-500/10 text-red-700 dark:text-red-300",
  neutral: "border-border bg-muted/45 text-foreground",
};

const PILL =
  "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md border px-2 text-xs font-medium";
const INTERACTIVE =
  "transition-opacity hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring";

export function ReadinessPill({
  tone,
  icon: Icon,
  title,
  description,
  href,
  onClick,
  busy = false,
  busyTitle,
}: {
  tone: ReadinessTone;
  icon: React.ComponentType<React.SVGProps<SVGSVGElement>>;
  title: string;
  /** Shown on hover: what the state means. */
  description?: string;
  /** Opens where the blocker is dealt with. */
  href?: string;
  /** Fixes the blocker in place (reprocessing journals). */
  onClick?: () => void;
  busy?: boolean;
  busyTitle?: string;
}) {
  const className = cn(PILL, readinessToneStyles[tone]);
  const content = (
    <>
      <Icon className={cn("size-4", busy && "animate-spin")} aria-hidden="true" />
      {busy && busyTitle ? busyTitle : title}
    </>
  );
  if (onClick) {
    return (
      <button
        type="button"
        className={cn(className, INTERACTIVE, "disabled:opacity-60")}
        title={description}
        onClick={onClick}
        disabled={busy}
      >
        {content}
      </button>
    );
  }
  if (href) {
    return (
      <Link to={href} className={cn(className, INTERACTIVE)} title={description}>
        {content}
      </Link>
    );
  }
  return (
    <span className={className} title={description}>
      {content}
    </span>
  );
}
