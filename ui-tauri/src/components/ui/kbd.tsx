import type * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Keycap, ported from T3Code's `ui/kbd`.
 *
 * Sans-serif rather than mono on purpose: a keycap sits inline with UI copy, and
 * a mono glyph at this size reads as code. Sized to hold either a short word
 * ("Esc", "Enter") or a single icon, hence `min-w-5` plus the icon size rule.
 */
function Kbd({ className, ...props }: React.ComponentProps<"kbd">) {
  return (
    <kbd
      data-slot="kbd"
      className={cn(
        // `rounded-sm`, not `rounded`: this app sets `--radius: 0`, so the bare
        // radius utility computes to 0px and the keycap comes out square.
        "pointer-events-none inline-flex h-5 min-w-5 items-center justify-center gap-1 rounded-sm bg-muted px-1 font-sans text-xs font-medium text-muted-foreground select-none [&_svg:not([class*='size-'])]:size-3",
        className,
      )}
      {...props}
    />
  );
}

/** A keycap (or several) with its label, e.g. `[↑][↓] Navigate`. */
function KbdGroup({ className, ...props }: React.ComponentProps<"kbd">) {
  return (
    <kbd
      data-slot="kbd-group"
      // `font-sans` because this is a <kbd> (matching T3Code's markup) and the
      // UA stylesheet gives <kbd> a monospace family — without this the label
      // text sitting beside the keycaps renders in the mono face.
      className={cn("inline-flex items-center gap-1 font-sans", className)}
      {...props}
    />
  );
}

export { Kbd, KbdGroup };
