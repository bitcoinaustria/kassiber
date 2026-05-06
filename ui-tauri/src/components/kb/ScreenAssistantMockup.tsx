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
import { Link } from "@tanstack/react-router";
import {
  ChevronDown,
  ChevronUp,
  Maximize2,
  MessageSquareText,
} from "lucide-react";

import Ai02 from "@/components/ai-02";
import { useAssistantSession } from "@/components/ai/assistantSession";
import { ChatThread } from "@/components/ai/ChatThread";
import { ToolConsentDialog } from "@/components/ai/ToolConsentDialog";
import { Button } from "@/components/ui/button";
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
  const [isThreadCollapsed, setIsThreadCollapsed] = React.useState(false);
  const {
    messages,
    isStreaming,
    abort,
    error,
    pendingConsent,
    sendConsent,
    selection,
    setSelection,
    thinkingEffort,
    setThinkingEffort,
    sendPrompt,
    toolsEnabled,
    setToolsEnabled,
  } = useAssistantSession();

  const compact = collapsed && !isInteracting && messages.length === 0;
  const hasThread = messages.length > 0;
  const showThread = hasThread && !isThreadCollapsed;
  const modelPickerEnabled = isInteracting || hasThread || isStreaming;

  React.useEffect(() => {
    if (!hasThread) {
      setIsThreadCollapsed(false);
    }
  }, [hasThread]);

  return (
    <section
      aria-label="Kassiber assistant"
      className={cn(
        "pointer-events-none relative z-30 px-3 pb-6 sm:px-4 sm:pb-7 md:px-6",
        className,
      )}
    >
      <div
        className={cn(
          "pointer-events-auto mx-auto flex w-full flex-col rounded-[28px] border border-white/70 bg-muted/85 p-2 shadow-[0_24px_90px_rgba(15,23,42,0.26),0_3px_18px_rgba(15,23,42,0.12),inset_0_1px_0_rgba(255,255,255,0.80)] ring-1 ring-zinc-950/10 backdrop-blur-2xl backdrop-saturate-150 transition-[max-width,transform] duration-200 ease-out dark:border-border dark:bg-card dark:shadow-[0_18px_48px_rgba(0,0,0,0.28)] dark:ring-border/70 dark:backdrop-blur-none dark:backdrop-saturate-100",
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
          <div className="min-h-0 overflow-hidden rounded-2xl border border-border/70 bg-background/92 shadow-none backdrop-blur-md">
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
                asChild
                variant="ghost"
                size="icon-xs"
                className="ml-auto rounded-full"
              >
                <Link
                  to="/assistant"
                  aria-label="Open assistant page"
                  title="Open assistant page"
                >
                  <Maximize2 className="h-3.5 w-3.5" aria-hidden="true" />
                </Link>
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                className="rounded-full"
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
          <div className="flex w-full items-center gap-1.5 rounded-2xl border border-border/70 bg-background/92 px-2 py-1.5 text-xs text-muted-foreground shadow-none">
            <button
              type="button"
              className="flex min-w-0 flex-1 items-center gap-2 rounded-xl px-1 py-1 text-left transition-colors hover:bg-muted/60"
              onClick={() => setIsThreadCollapsed(false)}
              aria-label="Expand preserved conversation"
            >
              <MessageSquareText
                className="h-3.5 w-3.5 shrink-0"
                aria-hidden="true"
              />
              <span className="font-medium text-foreground">
                Conversation hidden
              </span>
              <span className="min-w-0 flex-1 truncate">
                {messages.length}{" "}
                {messages.length === 1 ? "message" : "messages"} preserved for
                context
              </span>
              {isStreaming ? (
                <span className="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium uppercase text-primary">
                  Generating
                </span>
              ) : null}
              <ChevronUp className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
            </button>
            <Button
              asChild
              variant="ghost"
              size="icon-xs"
              className="rounded-full"
            >
              <Link
                to="/assistant"
                aria-label="Open assistant page"
                title="Open assistant page"
              >
                <Maximize2 className="h-3.5 w-3.5" aria-hidden="true" />
              </Link>
            </Button>
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
          onSubmit={sendPrompt}
          onAbort={abort}
          isStreaming={isStreaming}
          toolsEnabled={toolsEnabled}
          onToolsEnabledChange={setToolsEnabled}
          thinkingEffort={thinkingEffort}
          onThinkingEffortChange={setThinkingEffort}
          inputPanelElevated={false}
          modelPickerEnabled={modelPickerEnabled}
        />
        <ToolConsentDialog
          request={pendingConsent}
          onDecision={sendConsent}
        />
      </div>
    </section>
  );
}
