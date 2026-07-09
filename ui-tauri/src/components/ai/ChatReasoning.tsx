/**
 * Collapsible "thinking" pane for assistant reasoning content.
 *
 * Defaults collapsed so thinking stays available without taking over the
 * message while the answer streams. Users can expand it when useful.
 */

import * as React from "react";
import { Sparkles } from "lucide-react";
import { useTranslation } from "react-i18next";

import {
  ChainOfThought,
  ChainOfThoughtContent,
  ChainOfThoughtHeader,
} from "@/components/ai-elements";

import { ChatMarkdown } from "./ChatMarkdown";

interface ChatReasoningProps {
  thinking: string;
  isStreaming: boolean;
  hasAnswer: boolean;
  defaultOpen?: boolean;
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

function ThinkingHeaderLabel({ label }: { label: string }) {
  return (
    <span className="inline-flex min-w-0 items-center">
      <span className="thinking-label-active truncate">{label}</span>
      <span className="inline-flex w-[1.25em] shrink-0" aria-hidden="true">
        <span className="thinking-dot">.</span>
        <span className="thinking-dot">.</span>
        <span className="thinking-dot">.</span>
      </span>
    </span>
  );
}

export function ChatReasoning({
  thinking,
  isStreaming,
  hasAnswer,
  defaultOpen = false,
}: ChatReasoningProps) {
  const { t } = useTranslation("assistant");
  const [open, setOpen] = React.useState(defaultOpen);
  const [elapsedSeconds, setElapsedSeconds] = React.useState(0);
  const startedAt = React.useRef(Date.now());
  const wasStreamingRef = React.useRef(false);

  const streaming = isStreaming && !hasAnswer;

  React.useEffect(() => {
    if (streaming) {
      wasStreamingRef.current = true;
    }
  }, [streaming]);

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

  const durationLabel = elapsedSeconds > 0 ? `${elapsedSeconds}s` : "<1s";

  const headerLabel = streaming ? (
    <ThinkingHeaderLabel label={t("message.thinking")} />
  ) : wasStreamingRef.current ? (
    t("message.thoughtFor", { duration: durationLabel })
  ) : (
    t("message.thoughts")
  );

  return (
    <ChainOfThought
      open={open}
      onOpenChange={setOpen}
      className="mb-2 w-full min-w-0"
    >
      <ChainOfThoughtHeader icon={Sparkles}>{headerLabel}</ChainOfThoughtHeader>
      <ChainOfThoughtContent>
        <div className="mt-1 max-w-full border-l border-border/70 py-0.5 pl-3">
          <ChatMarkdown
            content={thinking}
            className={reasoningMarkdownClassName}
          />
        </div>
      </ChainOfThoughtContent>
    </ChainOfThought>
  );
}
