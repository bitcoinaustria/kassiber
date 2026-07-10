/**
 * Collapsible "thinking" pane(s) for assistant reasoning content.
 *
 * One assistant turn can contain several provider completion rounds
 * (think → tools → think again). Each round gets its own segment so
 * Ollama/oMLX reasoning stays split instead of one continuous blob.
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
import type { AiChatThinkingSegment } from "@/daemon/stream";

import { ChatMarkdown } from "./ChatMarkdown";

interface ChatReasoningProps {
  /** Preferred: one pane per model-round. */
  segments?: AiChatThinkingSegment[];
  /** Legacy single-string fallback when segments are absent. */
  thinking?: string;
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

function ThinkingSegmentPane({
  content,
  headerLabel,
  defaultOpen,
}: {
  content: string;
  headerLabel: React.ReactNode;
  defaultOpen: boolean;
}) {
  const [open, setOpen] = React.useState(defaultOpen);

  return (
    <ChainOfThought
      open={open}
      onOpenChange={setOpen}
      className="w-full min-w-0"
    >
      <ChainOfThoughtHeader icon={Sparkles}>{headerLabel}</ChainOfThoughtHeader>
      <ChainOfThoughtContent>
        <div className="mt-1 max-w-full border-l border-border/70 py-0.5 pl-3">
          <ChatMarkdown
            content={content}
            className={reasoningMarkdownClassName}
          />
        </div>
      </ChainOfThoughtContent>
    </ChainOfThought>
  );
}

export function ChatReasoning({
  segments,
  thinking,
  isStreaming,
  hasAnswer,
  defaultOpen = false,
}: ChatReasoningProps) {
  const { t } = useTranslation("assistant");
  const [elapsedSeconds, setElapsedSeconds] = React.useState(0);
  const startedAt = React.useRef(Date.now());
  const wasStreamingRef = React.useRef(false);

  let resolvedSegments: AiChatThinkingSegment[] = [];
  if (segments && segments.length > 0) {
    resolvedSegments = segments.filter((segment) => segment.content.length > 0);
  } else if (thinking) {
    resolvedSegments = [{ id: "legacy", content: thinking }];
  }

  const streaming = isStreaming && !hasAnswer;
  const activeIndex = streaming ? resolvedSegments.length - 1 : -1;

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

  if (resolvedSegments.length === 0) return null;

  const durationLabel = elapsedSeconds > 0 ? `${elapsedSeconds}s` : "<1s";
  const multi = resolvedSegments.length > 1;

  return (
    <div className="mb-2 flex w-full min-w-0 flex-col gap-1.5">
      {resolvedSegments.map((segment, index) => {
        const isActive = index === activeIndex;
        let headerLabel: React.ReactNode;
        if (isActive) {
          headerLabel = (
            <ThinkingHeaderLabel
              label={
                multi
                  ? t("message.thinkingRound", { round: index + 1 })
                  : t("message.thinking")
              }
            />
          );
        } else if (wasStreamingRef.current && !multi && index === 0) {
          headerLabel = t("message.thoughtFor", { duration: durationLabel });
        } else if (multi) {
          headerLabel = t("message.thoughtsRound", { round: index + 1 });
        } else {
          headerLabel = t("message.thoughts");
        }

        return (
          <ThinkingSegmentPane
            key={segment.id}
            content={segment.content}
            headerLabel={headerLabel}
            defaultOpen={defaultOpen}
          />
        );
      })}
    </div>
  );
}
