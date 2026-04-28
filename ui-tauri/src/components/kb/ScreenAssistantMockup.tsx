/**
 * Shell-wide assistant footer.
 *
 * Renders the Ai02 cosmetic input at the bottom of every authenticated
 * route plus a thread that grows above it once a conversation is active.
 * Streaming is handled through the daemon transport via `useAiChatStream`,
 * which feeds the `<think>` parser and writes message state.
 */

import * as React from "react";

import Ai02 from "@/components/ai-02";
import { ChatThread } from "@/components/ai/ChatThread";
import { useAiChatStream } from "@/daemon/stream";
import { cn } from "@/lib/utils";

interface ScreenAssistantMockupProps {
  className?: string;
  collapsed?: boolean;
}

export function ScreenAssistantMockup({
  className,
  collapsed = false,
}: ScreenAssistantMockupProps) {
  const [isInteracting, setIsInteracting] = React.useState(false);
  const [selection, setSelection] = React.useState<
    { provider: string; model: string } | null
  >(null);
  const [toolsEnabled, setToolsEnabled] = React.useState(true);

  const { messages, isStreaming, send, abort, error } = useAiChatStream();

  const compact = collapsed && !isInteracting && messages.length === 0;
  const hasThread = messages.length > 0;

  const handleSubmit = React.useCallback(
    (prompt: string) => {
      if (!selection?.model) return;
      const userMessages = messages
        .filter((message) => message.role !== "system")
        .map((message) => ({
          role: message.role,
          content: message.content,
        }));
      const next = [
        ...userMessages,
        { role: "user" as const, content: prompt },
      ];
      void send(
        {
          provider: selection.provider,
          model: selection.model,
          messages: next,
          toolsEnabled,
          toolLoopMaxIterations: 8,
          systemPromptKind: toolsEnabled ? "kassiber" : null,
        },
        prompt,
      );
    },
    [messages, selection, send, toolsEnabled],
  );

  return (
    <section
      aria-label="Kassiber assistant"
      className={cn("pointer-events-none px-3 pb-4 sm:px-4 md:px-6", className)}
    >
      <div
        className="pointer-events-auto mx-auto flex w-full max-w-3xl flex-col gap-3"
        onMouseEnter={() => setIsInteracting(true)}
        onMouseLeave={() => setIsInteracting(false)}
        onFocusCapture={() => setIsInteracting(true)}
        onBlurCapture={(event) => {
          if (!event.currentTarget.contains(event.relatedTarget)) {
            setIsInteracting(false);
          }
        }}
      >
        {hasThread ? (
          <ChatThread
            messages={messages}
            className="max-h-[calc(60vh-160px)]"
          />
        ) : null}
        {error ? (
          <div className="mx-auto w-full max-w-3xl rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {error.message}
          </div>
        ) : null}
        <Ai02
          compact={compact}
          selection={selection}
          onSelectionChange={setSelection}
          onSubmit={handleSubmit}
          onAbort={abort}
          isStreaming={isStreaming}
          toolsEnabled={toolsEnabled}
          onToolsEnabledChange={setToolsEnabled}
        />
      </div>
    </section>
  );
}
