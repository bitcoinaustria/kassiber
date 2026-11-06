import { Info } from "lucide-react";

import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { cn } from "@/lib/utils";

/**
 * A small info button whose notes open on click, tap, or keyboard, so they
 * are not hover-only.
 */
export function InfoNote({
  label,
  notes,
  className,
}: {
  /** Names the button for assistive technology. */
  label: string;
  notes: string[];
  className?: string;
}) {
  if (!notes.length) return null;
  return (
    <Popover>
      <PopoverTrigger asChild>
        <button
          type="button"
          aria-label={label}
          className={cn(
            "inline-flex shrink-0 items-center rounded-sm text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            className,
          )}
        >
          <Info className="size-3.5" aria-hidden="true" />
        </button>
      </PopoverTrigger>
      <PopoverContent align="start" className="w-72 space-y-1 p-3 text-xs">
        {notes.map((note) => (
          <p key={note}>{note}</p>
        ))}
      </PopoverContent>
    </Popover>
  );
}
