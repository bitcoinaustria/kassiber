/**
 * The composer toolbar's control look: model picker trigger, reasoning-effort
 * trigger and similar quiet pill buttons.
 *
 * Adapted from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
 * `apps/web/src/components/chat/ComposerControl.tsx`. Kassiber renders a plain
 * `<button>` (Radix `asChild` triggers forward their props onto it) instead of
 * Base UI's `useRender`.
 */

import type * as React from "react";
import { ChevronDown } from "lucide-react";

import { cn } from "@/lib/utils";

export const composerControlClassName =
  "relative inline-flex h-7 shrink-0 cursor-pointer items-center justify-center gap-1.5 whitespace-nowrap rounded-lg border border-transparent px-2 text-sm font-medium leading-none text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-60 data-[state=open]:bg-accent data-[state=open]:text-foreground [&:active:not([aria-haspopup])]:scale-[0.97] [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";

export function ComposerControl({
  className,
  type = "button",
  ...props
}: React.ComponentProps<"button">) {
  return (
    <button
      type={type}
      data-slot="composer-control"
      className={cn(composerControlClassName, className)}
      {...props}
    />
  );
}

export function ComposerControlChevron({ className }: { className?: string }) {
  return (
    <ChevronDown
      aria-hidden="true"
      strokeWidth={2.25}
      className={cn("size-3.5 shrink-0 opacity-60", className)}
    />
  );
}

export function ComposerControlSeparator({ className }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn("mx-0.5 hidden h-4 w-px shrink-0 bg-border sm:block", className)}
    />
  );
}
