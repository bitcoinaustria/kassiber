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
import { ChevronDown, ChevronUp, MessageSquareText } from "lucide-react";

import Ai02 from "@/components/ai-02";
import { ChatThread } from "@/components/ai/ChatThread";
import { ToolConsentDialog } from "@/components/ai/ToolConsentDialog";
import { Button } from "@/components/ui/button";
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
  const [isThreadCollapsed, setIsThreadCollapsed] = React.useState(false);

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
  const showThread = hasThread && !isThreadCollapsed;

  React.useEffect(() => {
    if (!hasThread) {
      setIsThreadCollapsed(false);
    }
  }, [hasThread]);

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
          showThread ? "max-w-5xl gap-2" : "max-w-3xl gap-3",
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
        {showThread ? (
          <div className="min-h-0 overflow-hidden rounded-2xl border border-border/70 bg-background/92 shadow-[0_10px_35px_rgba(15,23,42,0.12)] backdrop-blur-md">
            <div className="flex items-center gap-2 border-b border-border/60 px-3 py-2 text-xs text-muted-foreground">
              <MessageSquareText className="h-3.5 w-3.5" aria-hidden="true" />
              <span className="font-medium text-foreground">Conversation</span>
              <span>
                {messages.length} {messages.length === 1 ? "message" : "messages"}
              </span>
              {isStreaming ? (
                <span className="ml-1 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium uppercase text-primary">
                  Generating
                </span>
              ) : null}
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                className="ml-auto rounded-full"
                onClick={() => setIsThreadCollapsed(true)}
                aria-label="Collapse conversation"
                title="Collapse conversation"
              >
                <ChevronDown className="h-3.5 w-3.5" aria-hidden="true" />
              </Button>
            </div>
            <ChatThread
              messages={messages}
              className="max-h-[min(52vh,520px)] max-w-none p-3 pt-2"
            />
          </div>
        ) : null}
        {hasThread && isThreadCollapsed ? (
          <button
            type="button"
            className="flex w-full items-center gap-2 rounded-2xl border border-border/70 bg-background/92 px-3 py-2 text-left text-xs text-muted-foreground shadow-[0_10px_35px_rgba(15,23,42,0.12)] transition-colors hover:bg-background"
            onClick={() => setIsThreadCollapsed(false)}
            aria-label="Expand preserved conversation"
          >
            <MessageSquareText className="h-3.5 w-3.5" aria-hidden="true" />
            <span className="font-medium text-foreground">
              Conversation hidden
            </span>
            <span className="min-w-0 flex-1 truncate">
              {messages.length} {messages.length === 1 ? "message" : "messages"} preserved for context
            </span>
            {isStreaming ? (
              <span className="rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium uppercase text-primary">
                Generating
              </span>
            ) : null}
            <ChevronUp className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          </button>
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
