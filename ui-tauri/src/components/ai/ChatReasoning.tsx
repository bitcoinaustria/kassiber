/**
 * Collapsible "thinking" pane for assistant reasoning content.
 *
 * Default open while the model is mid-stream so the user can watch the
 * reasoning unfold; auto-collapses once the answer starts to land. Users
 * can expand again to re-read the chain of thought.
 */

import { ChevronDown, Sparkles } from "lucide-react";
import * as React from "react";

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

interface ChatReasoningProps {
  thinking: string;
  isStreaming: boolean;
  hasAnswer: boolean;
}

export function ChatReasoning({
  thinking,
  isStreaming,
  hasAnswer,
}: ChatReasoningProps) {
  // Auto-collapse the moment the answer text starts streaming.
  const autoOpen = isStreaming && !hasAnswer;
  const [open, setOpen] = React.useState(autoOpen);
  const lastAuto = React.useRef(autoOpen);

  React.useEffect(() => {
    if (lastAuto.current !== autoOpen) {
      setOpen(autoOpen);
      lastAuto.current = autoOpen;
    }
  }, [autoOpen]);

  if (!thinking) return null;

  return (
    <Collapsible open={open} onOpenChange={setOpen} className="mb-2">
      <CollapsibleTrigger
        className={cn(
          "group flex w-full items-center gap-2 rounded-md border border-border/50 bg-muted/40 px-2.5 py-1.5 text-xs text-muted-foreground transition-colors hover:bg-muted/70",
        )}
      >
        <Sparkles
          className={cn(
            "h-3.5 w-3.5 shrink-0",
            isStreaming && !hasAnswer && "animate-pulse text-primary",
          )}
          aria-hidden="true"
        />
        <span className="flex-1 text-left font-medium">
          {isStreaming && !hasAnswer ? "Thinking…" : "Thoughts"}
        </span>
        <ChevronDown
          className={cn(
            "h-3.5 w-3.5 shrink-0 transition-transform duration-200",
            open && "rotate-180",
          )}
          aria-hidden="true"
        />
      </CollapsibleTrigger>
      <CollapsibleContent className="data-[state=open]:animate-collapsible-down data-[state=closed]:animate-collapsible-up overflow-hidden">
        <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap rounded-md border border-border/40 bg-muted/30 p-3 font-mono text-[11px] leading-relaxed text-muted-foreground">
          {thinking}
        </pre>
      </CollapsibleContent>
    </Collapsible>
  );
}
