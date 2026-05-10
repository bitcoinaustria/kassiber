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

import { ChatMarkdown } from "./ChatMarkdown";

interface ChatReasoningProps {
  thinking: string;
  isStreaming: boolean;
  hasAnswer: boolean;
}

const reasoningMarkdownClassName = [
  "text-xs leading-6 text-muted-foreground",
  "[&_blockquote]:my-2 [&_blockquote]:pl-3",
  "[&_h1]:!mb-2 [&_h1]:!mt-3 [&_h1]:!text-sm",
  "[&_h2]:!mb-2 [&_h2]:!mt-3 [&_h2]:!text-sm",
  "[&_h3]:!mb-2 [&_h3]:!mt-3 [&_h3]:!text-xs",
  "[&_h4]:!my-2",
  "[&_ol]:my-2 [&_ul]:my-2",
  "[&_p]:my-2 [&_pre]:my-2 [&_table]:text-xs",
].join(" ");

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
        <div className="mt-2 max-w-full border-l border-border/70 py-1 pl-4">
          <ChatMarkdown
            content={thinking}
            className={reasoningMarkdownClassName}
          />
        </div>
      </ReasoningContent>
    </Reasoning>
  );
}
