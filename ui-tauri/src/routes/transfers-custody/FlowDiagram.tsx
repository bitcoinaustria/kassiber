import { ArrowRight, HelpCircle } from "lucide-react";

import { cn } from "@/lib/utils";

export interface FlowNode {
  label: string;
  sub?: string;
  /** Dashed styling for the unobserved hop (the wallet Kassiber never saw). */
  unknown?: boolean;
}

interface FlowEdgeProps {
  amount: string;
  dashed?: boolean;
  hideSensitive?: boolean;
}

function FlowEdge({ amount, dashed, hideSensitive }: FlowEdgeProps) {
  return (
    <div className="flex min-w-20 flex-1 flex-col items-center gap-0.5 self-center px-1">
      <span
        className={cn(
          "font-mono text-xs font-semibold tabular-nums",
          hideSensitive && "sensitive",
        )}
      >
        {amount}
      </span>
      <div className="flex w-full items-center text-muted-foreground/70">
        <div
          className={cn(
            "h-px flex-1 border-t",
            dashed ? "border-dashed" : "border-solid",
            "border-muted-foreground/50",
          )}
        />
        <ArrowRight className="-ml-1 size-3.5 shrink-0" aria-hidden="true" />
      </div>
    </div>
  );
}

function FlowNodeBox({ node, hideSensitive }: { node: FlowNode; hideSensitive?: boolean }) {
  return (
    <div
      className={cn(
        "min-w-32 rounded-md border px-3 py-2",
        node.unknown
          ? "border-dashed text-muted-foreground"
          : "bg-card",
      )}
    >
      <p
        className={cn(
          "flex items-center gap-1.5 text-sm font-semibold",
          hideSensitive && !node.unknown && "sensitive",
        )}
      >
        {node.unknown ? (
          <HelpCircle className="size-3.5 shrink-0" aria-hidden="true" />
        ) : null}
        {node.label}
      </p>
      {node.sub ? (
        <p className="mt-0.5 text-xs text-muted-foreground">{node.sub}</p>
      ) : null}
    </div>
  );
}

/**
 * The money-flow picture at the center of a decision card: source wallet →
 * (optional unobserved hop) → destination wallet, with the moved amounts on
 * the edges and fee / unexplained-remainder notes underneath.
 */
export function FlowDiagram({
  from,
  via,
  to,
  outAmount,
  backAmount,
  fee,
  residual,
  hideSensitive,
}: {
  from: FlowNode;
  via?: FlowNode;
  to: FlowNode;
  outAmount: string;
  /** Amount on the second edge; only used when `via` is present. */
  backAmount?: string;
  fee?: string | null;
  residual?: string | null;
  hideSensitive?: boolean;
}) {
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap items-stretch gap-y-2">
        <FlowNodeBox node={from} hideSensitive={hideSensitive} />
        <FlowEdge amount={outAmount} dashed={Boolean(via)} hideSensitive={hideSensitive} />
        {via ? (
          <>
            <FlowNodeBox node={via} hideSensitive={hideSensitive} />
            <FlowEdge
              amount={backAmount ?? ""}
              dashed
              hideSensitive={hideSensitive}
            />
          </>
        ) : null}
        <FlowNodeBox node={to} hideSensitive={hideSensitive} />
      </div>
      {fee || residual ? (
        <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs">
          {fee ? (
            <span className={cn("text-muted-foreground", hideSensitive && "sensitive")}>
              {fee}
            </span>
          ) : null}
          {residual ? (
            <span
              className={cn(
                "font-medium text-amber-700 dark:text-amber-300",
                hideSensitive && "sensitive",
              )}
            >
              ⚠ {residual}
            </span>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
