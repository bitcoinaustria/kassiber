/**
 * The frame the panel's own surfaces share, after T3 Code's panel surfaces:
 * a slim toolbar over a scrolling body, quiet section labels, compact rows
 * and a centred empty state.
 */

import * as React from "react";
import type { LucideIcon } from "lucide-react";

import { cn } from "@/lib/utils";

export function SurfaceFrame({
  label,
  toolbar,
  footer,
  children,
}: {
  /** The surface's accessible name. */
  label: string;
  /** What sits in the slim bar under the tabs: a summary, filters, actions. */
  toolbar?: React.ReactNode;
  footer?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section aria-label={label} className="flex min-h-0 flex-1 flex-col">
      {toolbar ? (
        <div className="flex min-h-9 shrink-0 items-center gap-2 border-b border-border/60 px-3 py-1.5 text-xs">
          {toolbar}
        </div>
      ) : null}
      <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>
      {footer ? (
        <div className="flex shrink-0 items-center gap-2 border-t border-border/60 px-3 py-2 text-xs">
          {footer}
        </div>
      ) : null}
    </section>
  );
}

export function SurfaceSection({
  title,
  count,
  children,
  className,
}: {
  title: string;
  count?: number;
  children: React.ReactNode;
  className?: string;
}) {
  const id = React.useId();
  return (
    <section aria-labelledby={id} className={cn("px-2 py-2", className)}>
      <h3
        id={id}
        className="flex items-center gap-1.5 px-2 pb-1 text-xs font-medium text-muted-foreground"
      >
        {title}
        {count !== undefined ? (
          <span className="tabular-nums text-muted-foreground/70">{count}</span>
        ) : null}
      </h3>
      {children}
    </section>
  );
}

export function SurfaceEmpty({
  icon: Icon,
  title,
  body,
  action,
  tone = "muted",
}: {
  icon: LucideIcon;
  title: string;
  body?: string;
  action?: React.ReactNode;
  tone?: "muted" | "success" | "destructive";
}) {
  return (
    <div
      className="flex h-full min-h-48 flex-col items-center justify-center gap-2 px-6 py-10 text-center"
      role={tone === "destructive" ? "alert" : "status"}
    >
      <span
        className={cn(
          "flex size-9 items-center justify-center rounded-full border",
          tone === "success" &&
            "border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400",
          tone === "destructive" && "border-destructive/30 bg-destructive/10 text-destructive",
          tone === "muted" && "border-border bg-muted/50 text-muted-foreground",
        )}
      >
        <Icon className="size-4" aria-hidden="true" />
      </span>
      <p className="text-sm font-medium text-foreground">{title}</p>
      {body ? <p className="max-w-64 text-xs text-muted-foreground">{body}</p> : null}
      {action}
    </div>
  );
}

/** A dot in the tone of what a row needs: red blocks reports, amber waits on review. */
export function ToneDot({ tone }: { tone: "blocker" | "review" | "muted" }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "size-1.5 shrink-0 rounded-full",
        tone === "blocker" && "bg-destructive",
        tone === "review" && "bg-amber-500",
        tone === "muted" && "bg-muted-foreground/50",
      )}
    />
  );
}

export const surfaceRowClassName =
  "flex w-full min-w-0 items-center gap-2.5 rounded-md px-2 py-1.5 text-left outline-none transition-colors hover:bg-accent/60 focus-visible:ring-2 focus-visible:ring-ring";
