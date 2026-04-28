import * as React from "react";
import type { LucideIcon } from "lucide-react";
import { ChevronDown, ListChecks } from "lucide-react";

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

function ChainOfThought({
  className,
  ...props
}: React.ComponentProps<typeof Collapsible>) {
  return (
    <Collapsible
      data-slot="chain-of-thought"
      className={cn("rounded-md border border-border/60 bg-muted/25", className)}
      {...props}
    />
  );
}

function ChainOfThoughtHeader({
  className,
  children = "Chain of thought",
  icon: Icon = ListChecks,
  ...props
}: React.ComponentProps<typeof CollapsibleTrigger> & {
  icon?: LucideIcon;
}) {
  return (
    <CollapsibleTrigger
      data-slot="chain-of-thought-header"
      className={cn(
        "group flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-xs text-muted-foreground transition-colors hover:bg-muted/45",
        className,
      )}
      {...props}
    >
      <Icon className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
      <span className="flex-1 text-left font-medium">{children}</span>
      <ChevronDown
        className="h-3.5 w-3.5 shrink-0 transition-transform duration-200 group-data-[state=open]:rotate-180"
        aria-hidden="true"
      />
    </CollapsibleTrigger>
  );
}

function ChainOfThoughtContent({
  className,
  ...props
}: React.ComponentProps<typeof CollapsibleContent>) {
  return (
    <CollapsibleContent
      data-slot="chain-of-thought-content"
      className={cn(
        "data-[state=open]:animate-collapsible-down data-[state=closed]:animate-collapsible-up overflow-hidden",
        className,
      )}
      {...props}
    />
  );
}

export {
  ChainOfThought,
  ChainOfThoughtContent,
  ChainOfThoughtHeader,
};
