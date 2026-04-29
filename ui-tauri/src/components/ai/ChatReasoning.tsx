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
  const [elapsedSeconds, setElapsedSeconds] = React.useState(0);
  const lastAuto = React.useRef(autoOpen);
  const startedAt = React.useRef(Date.now());

  React.useEffect(() => {
    if (lastAuto.current !== autoOpen) {
      setOpen(autoOpen);
      lastAuto.current = autoOpen;
    }
  }, [autoOpen]);

  React.useEffect(() => {
    if (!isStreaming || hasAnswer) return;
    startedAt.current = Date.now();
    setElapsedSeconds(0);
    const interval = window.setInterval(() => {
      setElapsedSeconds(Math.floor((Date.now() - startedAt.current) / 1000));
    }, 1000);
    return () => window.clearInterval(interval);
  }, [hasAnswer, isStreaming]);

  if (!thinking) return null;

  const elapsedLabel = elapsedSeconds > 0 ? `${elapsedSeconds}s` : "<1s";

  return (
    <Reasoning
      open={open}
      onOpenChange={setOpen}
      className="mb-4 w-full min-w-0"
    >
      <ReasoningTrigger isStreaming={isStreaming && !hasAnswer}>
        {isStreaming && !hasAnswer
          ? `Thinking ${elapsedLabel}`
          : "Thoughts"}
      </ReasoningTrigger>
      <ReasoningContent>
        <pre className="mt-2 max-w-full whitespace-pre-wrap break-words border-l border-border/70 py-1 pl-4 font-mono text-[11px] leading-relaxed text-muted-foreground">
          {thinking}
        </pre>
      </ReasoningContent>
    </Reasoning>
  );
}
