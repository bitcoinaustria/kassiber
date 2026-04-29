/**
 * Compact status indicator shown while the assistant is mid-stream and
 * has not yet emitted visible answer content.
 */

import { LoaderCircle } from "lucide-react";

import { cn } from "@/lib/utils";

interface ChatLoaderProps {
  className?: string;
  label?: string;
}

export function ChatLoader({ className, label = "Generating" }: ChatLoaderProps) {
  return (
    <div
      role="status"
      aria-label={`Assistant status: ${label}`}
      className={cn(
        "inline-flex items-center gap-2 rounded-full bg-muted/70 px-2.5 py-1 text-xs text-muted-foreground",
        className,
      )}
    >
      <LoaderCircle className="h-3.5 w-3.5 animate-spin" aria-hidden="true" />
      <span>{label}</span>
    </div>
  );
}
