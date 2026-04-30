import * as React from "react";
import { Download } from "lucide-react";

import Ai02 from "@/components/ai-02";
import { useAssistantSession } from "@/components/ai/assistantSession";
import { ChatThread } from "@/components/ai/ChatThread";
import { ToolConsentDialog } from "@/components/ai/ToolConsentDialog";
import { Button } from "@/components/ui/button";
import { saveChatExport } from "@/lib/chatExport";
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
  } = useAssistantSession();
  const hasMessages = messages.length > 0;
  const [exportStatus, setExportStatus] = React.useState<string | null>(null);

  const exportChat = React.useCallback(async () => {
    if (messages.length === 0) return;
    setExportStatus(null);
    try {
      const result = await saveChatExport(messages);
      setExportStatus(
        result === "saved"
          ? "Exported"
          : result === "download-started"
            ? "Download started"
            : "Export cancelled",
      );
    } catch {
      setExportStatus("Export failed");
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
      toolsEnabled={toolsEnabled}
      onToolsEnabledChange={setToolsEnabled}
      {...(hasMessages ? { prompts: [] } : {})}
    />
  );

  return (
    <div className="mx-auto flex min-h-full w-full max-w-6xl flex-col px-4 py-3 sm:px-6 sm:py-4">
      <section
        aria-label="Assistant conversation"
        className="flex min-h-full flex-1 flex-col bg-background"
      >
        {hasMessages ? (
          <div className="mx-auto flex w-full max-w-4xl items-center justify-end gap-2 px-1 pb-3">
            {isStreaming ? (
              <span className="mr-auto rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary">
                Generating
              </span>
            ) : (
              <span className="mr-auto text-xs text-muted-foreground">
                {messages.length} message{messages.length === 1 ? "" : "s"}
              </span>
            )}
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
            {exportStatus ? (
              <span className="text-xs text-muted-foreground">
                {exportStatus}
              </span>
            ) : null}
          </div>
        ) : null}

        <div
          className={cn(
            "flex-1",
            hasMessages ? "" : "grid min-h-[480px] place-items-center",
          )}
        >
          <div
            className={cn(
              "mx-auto w-full",
              hasMessages
                ? "flex max-w-4xl flex-col"
                : "flex max-w-3xl -translate-y-6 flex-col items-center gap-7 px-1 text-center sm:-translate-y-10",
            )}
          >
            {hasMessages ? (
              <ChatThread
                messages={messages}
                scrollable={false}
                className="p-1 sm:p-2"
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
          <div className="mx-auto mt-4 w-full max-w-4xl shrink-0 border-t border-border/60 pt-3">
            {composer}
          </div>
        ) : null}
      </section>

      <ToolConsentDialog request={pendingConsent} onDecision={sendConsent} />
    </div>
  );
}
