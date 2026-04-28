import { Link } from "@tanstack/react-router";
import { Bot, Minimize2, PanelBottomOpen } from "lucide-react";

import Ai02 from "@/components/ai-02";
import { useAssistantSession } from "@/components/ai/assistantSession";
import { ChatThread } from "@/components/ai/ChatThread";
import { ToolConsentDialog } from "@/components/ai/ToolConsentDialog";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

export function Assistant() {
  const {
    messages,
    isStreaming,
    error,
    pendingConsent,
    selection,
    setSelection,
    sendPrompt,
    toolsEnabled,
    setToolsEnabled,
    sendConsent,
    abort,
    returnPath,
  } = useAssistantSession();
  const hasMessages = messages.length > 0;

  return (
    <div className="mx-auto flex min-h-full w-full max-w-6xl flex-col gap-4 px-4 py-4 sm:px-6 sm:py-5">
      <div className="flex flex-wrap items-center gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <Bot className="size-5 text-muted-foreground" aria-hidden="true" />
          <h2 className="text-lg font-medium">Assistant</h2>
          {isStreaming ? (
            <span className="rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary">
              Generating
            </span>
          ) : null}
        </div>

        <div className="ml-auto flex items-center gap-2">
          <Button asChild variant="outline" size="sm">
            <Link to={returnPath}>
              <PanelBottomOpen className="size-4" aria-hidden="true" />
              Docked chat
            </Link>
          </Button>
        </div>
      </div>

      <section
        aria-label="Assistant conversation"
        className={cn(
          "min-h-0 flex-1 overflow-hidden rounded-2xl border border-border/80 bg-background/95 shadow-[0_16px_50px_rgba(15,23,42,0.08)]",
          hasMessages ? "flex" : "grid place-items-center",
        )}
      >
        {hasMessages ? (
          <ChatThread messages={messages} className="h-full max-w-none p-4" />
        ) : (
          <div className="flex max-w-sm flex-col items-center gap-2 px-6 py-16 text-center">
            <Minimize2 className="size-7 text-muted-foreground" aria-hidden="true" />
            <p className="text-sm font-medium">No conversation yet</p>
            <p className="text-sm text-muted-foreground">
              Start with a question below, or switch back to the docked chat.
            </p>
          </div>
        )}
      </section>

      {error ? (
        <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error.message}
        </div>
      ) : null}

      <Ai02
        className="max-w-none"
        selection={selection}
        onSelectionChange={setSelection}
        onSubmit={sendPrompt}
        onAbort={abort}
        isStreaming={isStreaming}
        toolsEnabled={toolsEnabled}
        onToolsEnabledChange={setToolsEnabled}
      />
      <ToolConsentDialog request={pendingConsent} onDecision={sendConsent} />
    </div>
  );
}
