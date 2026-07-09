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
  iconClassName,
  ...props
}: React.ComponentProps<typeof CollapsibleTrigger> & {
  icon?: LucideIcon;
  iconClassName?: string;
}) {
  return (
    <CollapsibleTrigger
      data-slot="chain-of-thought-header"
      className={cn(
        "group inline-flex max-w-full min-w-0 items-center gap-1.5 rounded-md py-0.5 text-xs text-muted-foreground transition-all duration-100 ease-out hover:text-foreground active:scale-[0.99]",
        className,
      )}
      {...props}
    >
      <Icon
        className={cn("h-3 w-3 shrink-0", iconClassName)}
        aria-hidden="true"
      />
      <span className="min-w-0 flex-1 truncate text-left font-medium">{children}</span>
      <ChevronDown
        className="h-3 w-3 shrink-0 transition-transform duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] group-data-[state=open]:rotate-180 motion-reduce:transition-none"
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
