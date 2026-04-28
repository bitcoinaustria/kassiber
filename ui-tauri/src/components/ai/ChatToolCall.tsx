import {
  CheckCircle2,
  LoaderCircle,
  ShieldAlert,
  Wrench,
  XCircle,
} from "lucide-react";

import type { AiChatToolCall } from "@/daemon/stream";
import { cn } from "@/lib/utils";

interface ChatToolCallProps {
  toolCall: AiChatToolCall;
}
function statusLabel(status: AiChatToolCall["status"]): string {
  switch (status) {
    case "pending":
      return "Pending";
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

function StatusIcon({ status }: { status: AiChatToolCall["status"] }) {
  if (status === "done") {
    return <CheckCircle2 className="h-3.5 w-3.5 text-emerald-600" aria-hidden="true" />;
  }
  if (status === "denied") {
    return <ShieldAlert className="h-3.5 w-3.5 text-amber-600" aria-hidden="true" />;
  }
  if (status === "error") {
    return <XCircle className="h-3.5 w-3.5 text-destructive" aria-hidden="true" />;
  }
  if (status === "running") {
    return (
      <LoaderCircle
        className="h-3.5 w-3.5 animate-spin text-muted-foreground"
        aria-hidden="true"
      />
    );
  }
  return <Wrench className="h-3.5 w-3.5 text-muted-foreground" aria-hidden="true" />;
}

export function ChatToolCall({ toolCall }: ChatToolCallProps) {
  const hasArguments = Object.keys(toolCall.arguments).length > 0;
  return (
    <div
      className={cn(
        "mt-2 rounded-md border px-2.5 py-2 text-xs",
        toolCall.status === "error"
          ? "border-destructive/35 bg-destructive/5"
          : "border-border/70 bg-muted/35",
      )}
    >
      <div className="flex min-w-0 items-center gap-2">
        <StatusIcon status={toolCall.status} />
        <code className="min-w-0 flex-1 truncate font-mono text-[11px] text-foreground">
          {toolCall.name}
        </code>
        <span className="shrink-0 rounded-full bg-background px-1.5 py-0.5 text-[10px] font-medium uppercase text-muted-foreground">
          {statusLabel(toolCall.status)}
        </span>
      </div>
      {hasArguments ? (
        <pre className="mt-1 max-h-24 overflow-auto whitespace-pre-wrap break-words rounded bg-background/75 px-2 py-1 font-mono text-[10px] text-muted-foreground">
          {JSON.stringify(toolCall.arguments, null, 2)}
        </pre>
      ) : null}
      {toolCall.reason ? (
        <p className="mt-1 break-words font-mono text-[10px] text-muted-foreground">
          {toolCall.reason}
        </p>
      ) : null}
    </div>
  );
}
