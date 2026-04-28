/**
 * Shell-wide assistant footer.
 *
 * Renders the Ai02 input at the bottom of every authenticated route. Once a
 * conversation starts, the same dock grows upward and contains the scrollable
 * thread above the composer so the assistant feels like one expanded chatbox.
 * Streaming is handled through the daemon transport via `useAiChatStream`,
 * which feeds the `<think>` parser and writes message state.
 */

import * as React from "react";

import Ai02 from "@/components/ai-02";
import { ChatThread } from "@/components/ai/ChatThread";
import { ToolConsentDialog } from "@/components/ai/ToolConsentDialog";
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

  const {
    messages,
    isStreaming,
    send,
    abort,
    error,
    pendingConsent,
    sendConsent,
  } = useAiChatStream();

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
        className={cn(
          "pointer-events-auto mx-auto flex w-full flex-col rounded-[28px] border border-zinc-300/90 bg-zinc-200/78 p-2 shadow-[0_24px_90px_rgba(15,23,42,0.30),0_3px_18px_rgba(15,23,42,0.14)] ring-1 ring-white/90 backdrop-blur-xl transition-[max-width,transform] duration-200 ease-out dark:border-white/10 dark:bg-zinc-900/55 dark:ring-white/10",
          hasThread ? "max-w-5xl gap-2" : "max-w-3xl gap-3",
        )}
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
          <div className="min-h-0 overflow-hidden rounded-2xl border border-border/70 bg-background/92 shadow-[0_10px_35px_rgba(15,23,42,0.12)] backdrop-blur-md">
            <ChatThread
              messages={messages}
              className="max-h-[min(52vh,520px)] max-w-none p-3"
            />
          </div>
        ) : null}
        {error ? (
          <div className="w-full rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {error.message}
          </div>
        ) : null}
        <Ai02
          className="max-w-none border-0 bg-transparent p-0 shadow-none ring-0 backdrop-blur-0"
          compact={compact}
          selection={selection}
          onSelectionChange={setSelection}
          onSubmit={handleSubmit}
          onAbort={abort}
          isStreaming={isStreaming}
          toolsEnabled={toolsEnabled}
          onToolsEnabledChange={setToolsEnabled}
        />
        <ToolConsentDialog
          request={pendingConsent}
          onDecision={sendConsent}
        />
      </div>
    </section>
  );
}
