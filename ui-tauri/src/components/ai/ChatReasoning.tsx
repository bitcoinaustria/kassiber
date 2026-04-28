/**
 * Collapsible "thinking" pane for assistant reasoning content.
 *
 * Default open while the model is mid-stream so the user can watch the
 * reasoning unfold; auto-collapses once the answer starts to land. Users
 * can expand again to re-read the chain of thought.
 */

import * as React from "react";

import {
  Reasoning,
  ReasoningContent,
  ReasoningTrigger,
} from "@/components/ai-elements";

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
    <Reasoning open={open} onOpenChange={setOpen} className="mb-2">
      <ReasoningTrigger isStreaming={isStreaming && !hasAnswer}>
        {isStreaming && !hasAnswer ? "Thinking..." : "Thoughts"}
      </ReasoningTrigger>
      <ReasoningContent>
        <pre className="mt-2 max-h-64 overflow-auto whitespace-pre-wrap rounded-md border border-border/40 bg-muted/30 p-3 font-mono text-[11px] leading-relaxed text-muted-foreground">
          {thinking}
        </pre>
      </ReasoningContent>
    </Reasoning>
  );
}
