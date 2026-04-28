import * as React from "react";
import {
  CheckCircle2,
  ChevronDown,
  LoaderCircle,
  ShieldAlert,
  ShieldCheck,
  Wrench,
  XCircle,
} from "lucide-react";

import { Badge } from "@/components/ui/badge";
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

type ToolState =
  | "pending"
  | "awaiting_consent"
  | "running"
  | "done"
  | "denied"
  | "error";

function Tool({
  className,
  ...props
}: React.ComponentProps<typeof Collapsible>) {
  return (
    <Collapsible
      data-slot="tool"
      className={cn(
        "rounded-md border text-xs",
        "border-border/70 bg-background/70",
        className,
      )}
      {...props}
    />
  );
}

function ToolHeader({
  className,
  name,
  state,
  ...props
}: React.ComponentProps<typeof CollapsibleTrigger> & {
  name: string;
  state: ToolState;
}) {
  const Icon = iconForState(state);

  return (
    <CollapsibleTrigger
      data-slot="tool-header"
      className={cn(
        "group flex w-full min-w-0 items-center gap-2 rounded-md px-2.5 py-2 text-left transition-colors hover:bg-muted/45",
        className,
      )}
      {...props}
    >
      <Icon
        className={cn(
          "h-3.5 w-3.5 shrink-0",
          state === "done" && "text-emerald-600",
          state === "awaiting_consent" && "text-primary",
          state === "denied" && "text-amber-600",
          state === "error" && "text-destructive",
          state === "running" && "animate-spin text-muted-foreground",
          state === "pending" && "text-muted-foreground",
        )}
        aria-hidden="true"
      />
      <code className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground">
        {name}
      </code>
      <Badge
        variant={state === "error" ? "destructive" : "secondary"}
        className="shrink-0 text-[10px] uppercase"
      >
        {labelForState(state)}
      </Badge>
      <ChevronDown
        className="h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform duration-200 group-data-[state=open]:rotate-180"
        aria-hidden="true"
      />
    </CollapsibleTrigger>
  );
}

function ToolContent({
  className,
  ...props
}: React.ComponentProps<typeof CollapsibleContent>) {
  return (
    <CollapsibleContent
      data-slot="tool-content"
      className={cn(
        "data-[state=open]:animate-collapsible-down data-[state=closed]:animate-collapsible-up overflow-hidden",
        className,
      )}
      {...props}
    />
  );
}

function ToolInput({
  className,
  label = "Arguments",
  input,
  ...props
}: React.ComponentProps<"section"> & {
  input: unknown;
  label?: string;
}) {
  return (
    <section
      data-slot="tool-input"
      className={cn("border-t border-border/60 px-2.5 py-2", className)}
      {...props}
    >
      <p className="mb-1 text-[10px] font-medium uppercase text-muted-foreground">
        {label}
      </p>
      <pre className="max-h-28 overflow-auto whitespace-pre-wrap break-words rounded bg-muted/55 px-2 py-1 font-mono text-[10px] text-muted-foreground">
        {JSON.stringify(input, null, 2)}
      </pre>
    </section>
  );
}

function ToolOutput({
  className,
  label = "Result",
  output,
  error,
  ...props
}: React.ComponentProps<"section"> & {
  output?: unknown;
  error?: string;
  label?: string;
}) {
  return (
    <section
      data-slot="tool-output"
      className={cn("border-t border-border/60 px-2.5 py-2", className)}
      {...props}
    >
      <p className="mb-1 text-[10px] font-medium uppercase text-muted-foreground">
        {error ? "Error" : label}
      </p>
      <pre
        className={cn(
          "max-h-32 overflow-auto whitespace-pre-wrap break-words rounded px-2 py-1 font-mono text-[10px]",
          error
            ? "bg-destructive/10 text-destructive"
            : "bg-muted/55 text-muted-foreground",
        )}
      >
        {error ?? formatPayload(output)}
      </pre>
    </section>
  );
}

function formatPayload(payload: unknown): string {
  if (typeof payload === "string") return payload;
  return JSON.stringify(payload, null, 2);
}

function iconForState(state: ToolState) {
  switch (state) {
    case "pending":
      return Wrench;
    case "awaiting_consent":
      return ShieldCheck;
    case "running":
      return LoaderCircle;
    case "done":
      return CheckCircle2;
    case "denied":
      return ShieldAlert;
    case "error":
      return XCircle;
  }
}

function labelForState(state: ToolState): string {
  switch (state) {
    case "pending":
      return "Pending";
    case "awaiting_consent":
      return "Awaiting consent";
    case "running":
      return "Running";
    case "done":
      return "Done";
    case "denied":
      return "Denied";
    case "error":
      return "Error";
  }
}

export { Tool, ToolContent, ToolHeader, ToolInput, ToolOutput, type ToolState };
