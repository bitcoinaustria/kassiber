import * as React from "react";
import { ChevronDown, Sparkles } from "lucide-react";

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

function Reasoning({
  className,
  ...props
}: React.ComponentProps<typeof Collapsible>) {
  return (
    <Collapsible
      data-slot="reasoning"
      className={cn("w-full min-w-0 text-muted-foreground", className)}
      {...props}
    />
  );
}

function ReasoningTrigger({
  className,
  children,
  isStreaming = false,
  ...props
}: React.ComponentProps<typeof CollapsibleTrigger> & {
  isStreaming?: boolean;
}) {
  return (
    <CollapsibleTrigger
      data-slot="reasoning-trigger"
      className={cn(
        "group inline-flex max-w-full min-w-0 items-center gap-2 rounded-md py-1 text-sm text-muted-foreground transition-colors hover:text-foreground",
        className,
      )}
      {...props}
    >
      <Sparkles
        className={cn(
          "h-3.5 w-3.5 shrink-0",
          isStreaming && "animate-pulse text-primary",
        )}
        aria-hidden="true"
      />
      <span className="min-w-0 flex-1 truncate text-left font-medium">{children}</span>
      <ChevronDown
        className="h-3.5 w-3.5 shrink-0 transition-transform duration-200 group-data-[state=open]:rotate-180"
        aria-hidden="true"
      />
    </CollapsibleTrigger>
  );
}

function ReasoningContent({
  className,
  ...props
}: React.ComponentProps<typeof CollapsibleContent>) {
  return (
    <CollapsibleContent
      data-slot="reasoning-content"
      className={cn(
        "data-[state=open]:animate-collapsible-down data-[state=closed]:animate-collapsible-up overflow-hidden",
        className,
      )}
      {...props}
    />
  );
}

export { Reasoning, ReasoningContent, ReasoningTrigger };
