import * as React from "react";
import { Download, EyeOff, Trash2 } from "lucide-react";

import Ai02 from "@/components/ai-02";
import { useAssistantSession } from "@/components/ai/assistantSession";
import { ChatHistoryPanel } from "@/components/ai/ChatHistoryPanel";
import { ChatThread } from "@/components/ai/ChatThread";
import { ToolConsentDialog } from "@/components/ai/ToolConsentDialog";
import { useSupportedReasoningEffort } from "@/components/ai/useReasoningEffortSupport";
import { Button } from "@/components/ui/button";
import { saveChatExport } from "@/lib/chatExport";
import { cn } from "@/lib/utils";

export function Assistant() {
  const {
    messages,
    isStreaming,
    error,
    pendingConsent,
    queuedPrompts,
    selection,
    setSelection,
    thinkingEffort,
    setThinkingEffort,
    sendPrompt,
    sendConsent,
    abort,
    reset,
    incognito,
    setIncognito,
  } = useAssistantSession();
  const hasMessages = messages.length > 0;
  const queuedPromptCount = queuedPrompts.length;
  const supportsThinkingEffort = useSupportedReasoningEffort({
    selection,
    thinkingEffort,
    setThinkingEffort,
  });

  const exportChat = React.useCallback(async () => {
    if (messages.length === 0) return;
    try {
      await saveChatExport(messages);
    } catch {
      // Keep the toolbar stable; the save dialog itself owns completion state.
    }
  }, [messages]);

  const composer = (
    <Ai02
      className={cn(
        "max-w-none rounded-[28px] border-border/80 bg-muted/75 shadow-[0_18px_55px_rgba(15,23,42,0.16)] ring-0",
        hasMessages &&
          "rounded-[24px] border-border bg-background! shadow-none! ring-0!",
      )}
      selection={selection}
      onSelectionChange={setSelection}
      onSubmit={sendPrompt}
      onAbort={abort}
      isStreaming={isStreaming}
      thinkingEffort={thinkingEffort}
      onThinkingEffortChange={
        supportsThinkingEffort ? setThinkingEffort : undefined
      }
      showThinkingEffort={supportsThinkingEffort}
      {...(hasMessages ? { prompts: [] } : {})}
    />
  );

  return (
    <div
      className={cn(
        "mx-auto flex w-full max-w-6xl flex-col px-4 sm:px-6",
        hasMessages
          ? "h-full min-h-0 pt-3 pb-0 sm:pt-4"
          : "min-h-full py-3 sm:py-4",
      )}
    >
      <section
        aria-label="Assistant conversation"
        className="flex min-h-0 flex-1 flex-col bg-background"
      >
        <div className="mx-auto flex w-full max-w-4xl shrink-0 items-center justify-end gap-2 px-1 pb-3">
          {hasMessages ? (
            isStreaming ? (
              <span className="mr-auto rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary">
                Generating
                {queuedPromptCount > 0 ? ` - ${queuedPromptCount} queued` : ""}
              </span>
            ) : (
              <span className="mr-auto text-xs text-muted-foreground">
                {messages.length} message{messages.length === 1 ? "" : "s"}
              </span>
            )
          ) : (
            <span className="mr-auto" />
          )}
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className={cn(
              "gap-2",
              incognito
                ? "bg-primary/10 text-primary"
                : "text-muted-foreground",
            )}
            aria-pressed={incognito}
            onClick={() => setIncognito(!incognito)}
            title="Incognito: do not store this conversation"
          >
            <EyeOff className="size-4" aria-hidden="true" />
            Incognito
          </Button>
          <ChatHistoryPanel />
          {hasMessages ? (
            <>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="gap-2"
                onClick={exportChat}
              >
                <Download className="size-4" aria-hidden="true" />
                Export chat
              </Button>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="gap-2 text-muted-foreground hover:text-destructive"
                onClick={reset}
              >
                <Trash2 className="size-4" aria-hidden="true" />
                Clear chat
              </Button>
            </>
          ) : null}
        </div>

        <div
          className={cn(
            "flex-1",
            hasMessages
              ? "min-h-0 overflow-hidden"
              : "grid min-h-[480px] place-items-center",
          )}
        >
          <div
            className={cn(
              "mx-auto w-full",
              hasMessages
                ? "flex h-full min-h-0 max-w-4xl flex-col"
                : "flex max-w-3xl -translate-y-6 flex-col items-center gap-7 px-1 text-center sm:-translate-y-10",
            )}
          >
            {hasMessages ? (
              <ChatThread
                messages={messages}
                className="min-h-0 p-1 sm:p-2"
              />
            ) : (
              <>
                <h2 className="text-3xl font-medium tracking-normal text-foreground sm:text-4xl">
                  What needs reviewing today?
                </h2>
                <div className="w-full">{composer}</div>
              </>
            )}
          </div>
        </div>

        {error ? (
          <div className="mx-auto mt-3 w-full max-w-4xl rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error.message}
          </div>
        ) : null}

        {hasMessages ? (
          <div className="sticky bottom-0 z-20 -mx-4 mt-3 shrink-0 border-t border-border/60 bg-background/95 px-4 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] backdrop-blur sm:-mx-6 sm:px-6">
            <div className="mx-auto w-full max-w-4xl">{composer}</div>
          </div>
        ) : null}
      </section>

      <ToolConsentDialog request={pendingConsent} onDecision={sendConsent} />
    </div>
  );
}
