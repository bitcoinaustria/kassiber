/**
 * Three-dot typing indicator shown while the assistant is mid-stream and
 * has not yet emitted any visible-answer content.
 */

import { cn } from "@/lib/utils";

interface ChatLoaderProps {
  className?: string;
}

export function ChatLoader({ className }: ChatLoaderProps) {
  return (
    <div
      role="status"
      aria-label="Assistant is thinking"
      className={cn(
        "inline-flex items-center gap-1 text-muted-foreground",
        className,
      )}
    >
      <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-current [animation-delay:-0.3s]" />
      <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-current [animation-delay:-0.15s]" />
      <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-current" />
    </div>
  );
}
