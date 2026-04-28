import * as React from "react";
import type { LucideIcon } from "lucide-react";
import {
  CheckCircle2,
  ChevronDown,
  Circle,
  LoaderCircle,
  ListChecks,
} from "lucide-react";

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

type ChainOfThoughtStepStatus = "complete" | "active" | "pending";

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

function ChainOfThoughtStep({
  className,
  icon,
  label,
  description,
  status = "pending",
  children,
  ...props
}: React.ComponentProps<"div"> & {
  icon?: LucideIcon;
  label?: string;
  description?: string;
  status?: ChainOfThoughtStepStatus;
}) {
  const Icon = icon ?? statusIcon(status);

  return (
    <div
      data-slot="chain-of-thought-step"
      className={cn("flex gap-2 px-2.5 py-2 text-xs", className)}
      {...props}
    >
      <Icon
        className={cn(
          "mt-0.5 h-3.5 w-3.5 shrink-0",
          status === "complete" && "text-emerald-600",
          status === "active" && "animate-spin text-primary",
          status === "pending" && "text-muted-foreground",
        )}
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1 space-y-1">
        {label ? <p className="font-medium text-foreground">{label}</p> : null}
        {description ? (
          <p className="text-muted-foreground">{description}</p>
        ) : null}
        {children}
      </div>
    </div>
  );
}

function statusIcon(status: ChainOfThoughtStepStatus): LucideIcon {
  if (status === "complete") return CheckCircle2;
  if (status === "active") return LoaderCircle;
  return Circle;
}

export {
  ChainOfThought,
  ChainOfThoughtContent,
  ChainOfThoughtHeader,
  ChainOfThoughtStep,
  type ChainOfThoughtStepStatus,
};
