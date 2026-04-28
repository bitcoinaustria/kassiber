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
      className={cn(
        "w-full min-w-0 overflow-hidden text-muted-foreground",
        className,
      )}
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
        "group inline-flex max-w-full min-w-0 items-center gap-2 rounded-md py-1 text-sm text-muted-foreground transition-colors hover:text-foreground",
        className,
      )}
      {...props}
    >
      <Icon className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
      <span className="min-w-0 flex-1 truncate text-left font-medium">{children}</span>
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
