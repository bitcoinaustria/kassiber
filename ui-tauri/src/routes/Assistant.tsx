import * as React from "react";
import { useTranslation } from "react-i18next";
import { Check, Download, EyeOff, MoreHorizontal, Trash2 } from "lucide-react";

import Ai02 from "@/components/ai-02";
import { useAssistantSession } from "@/components/ai/assistantSession";
import { ChatHistoryPanel } from "@/components/ai/ChatHistoryPanel";
import { ChatThread } from "@/components/ai/ChatThread";
import { ToolConsentDialog } from "@/components/ai/ToolConsentDialog";
import { useSupportedReasoningEffort } from "@/components/ai/useReasoningEffortSupport";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { saveChatExport } from "@/lib/chatExport";
import { cn } from "@/lib/utils";
import { useAssistantDraftStore } from "@/store/assistantDraft";

export function Assistant() {
  const { t } = useTranslation("assistant");
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
    branchFromMessage,
    editUserMessage,
    incognito,
    setIncognito,
    sessionId,
  } = useAssistantSession();
  const assistantDraft = useAssistantDraftStore((s) => s.draft);
  const setAssistantDraft = useAssistantDraftStore((s) => s.setDraft);
  const hasMessages = messages.length > 0;
  const queuedPromptCount = queuedPrompts.length;
  const showIncognitoToggle = sessionId === null;
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
      // No outer card: the composer is a single borderless box in a shade
      // above the page background, with the suggestion chips sitting below it
      // (outside the box), mirroring the reference layout.
      className={cn(
        "max-w-none border-0 bg-transparent p-0 shadow-none ring-0 backdrop-blur-0",
        !hasMessages && "gap-4",
      )}
      composerClassName={cn(
        "border-0 bg-muted shadow-none backdrop-blur-0 dark:bg-muted",
        // The empty "agenda" hero gets a larger, squarer box to sit with the
        // suggestion chips; the docked-thread composer stays compact.
        hasMessages ? "rounded-2xl" : "min-h-[112px] rounded-3xl",
      )}
      alwaysShowSuggestions={!hasMessages}
      selection={selection}
      onSelectionChange={setSelection}
      value={assistantDraft}
      onValueChange={setAssistantDraft}
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
        "flex w-full flex-col bg-background",
        hasMessages ? "h-full min-h-0" : "min-h-full pb-3 sm:pb-4",
      )}
    >
      {/* Full-width toolbar: sibling to the centered conversation column, not capped by max-w-6xl. */}
      <div className="flex w-full shrink-0 items-center justify-end gap-2 px-4 pb-3 pt-3 sm:px-6 sm:pt-4">
        {hasMessages ? (
          isStreaming ? (
            <span className="mr-auto min-w-0 truncate rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary">
              {queuedPromptCount > 0
                ? t("page.generatingQueued", { count: queuedPromptCount })
                : t("page.generating")}
            </span>
          ) : (
            <span className="mr-auto min-w-0 truncate text-xs text-muted-foreground">
              {t("page.messageCount", { count: messages.length })}
            </span>
          )
        ) : (
          <span className="mr-auto" />
        )}
        {hasMessages ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="gap-2"
            onClick={exportChat}
          >
            <Download className="size-4" aria-hidden="true" />
            {t("page.exportChat")}
          </Button>
        ) : null}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={t("page.moreActions")}
              title={t("page.moreActions")}
            >
              <MoreHorizontal className="size-4" aria-hidden="true" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-52">
            {showIncognitoToggle ? (
              <DropdownMenuItem
                onSelect={(event) => {
                  // Keep the menu open so the toggle state is visible.
                  event.preventDefault();
                  setIncognito(!incognito);
                }}
                title={t("page.incognitoTitle")}
              >
                <EyeOff className="size-4" aria-hidden="true" />
                {t("page.incognito")}
                {incognito ? (
                  <Check
                    className="ml-auto size-4 text-primary"
                    aria-hidden="true"
                  />
                ) : null}
              </DropdownMenuItem>
            ) : null}
            <ChatHistoryPanel />
            {hasMessages ? (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem variant="destructive" onSelect={reset}>
                  <Trash2 className="size-4" aria-hidden="true" />
                  {t("page.clearChat")}
                </DropdownMenuItem>
              </>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <section
        aria-label={t("page.conversationLabel")}
        className="flex min-h-0 flex-1 flex-col"
      >
        <div
          className={cn(
            "flex-1",
            hasMessages
              ? "min-h-0 overflow-hidden"
              : "grid min-h-[480px] place-items-center px-4 sm:px-6",
          )}
        >
          {hasMessages ? (
            <ChatThread
              messages={messages}
              className="h-full min-h-0"
              contentClassName="mx-auto w-full max-w-4xl px-4 sm:px-6"
              onBranchMessage={branchFromMessage}
              onEditMessage={editUserMessage}
            />
          ) : (
            <div className="mx-auto flex w-full max-w-3xl -translate-y-6 flex-col items-center gap-7 px-1 text-center sm:-translate-y-10">
              <h2 className="text-3xl font-medium tracking-normal text-foreground sm:text-4xl">
                {t("page.heading")}
              </h2>
              <div className="w-full">{composer}</div>
            </div>
          )}
        </div>

        {error ? (
          <div className="mx-auto mt-3 w-full max-w-4xl px-4 sm:px-6">
            <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
              {error.message}
            </div>
          </div>
        ) : null}

        {hasMessages ? (
          <div className="sticky bottom-0 z-20 mx-auto mt-3 w-full max-w-4xl shrink-0 bg-gradient-to-t from-background via-background/95 to-transparent px-4 pt-6 pb-[max(0.75rem,env(safe-area-inset-bottom))] sm:px-6">
            {composer}
          </div>
        ) : null}
      </section>

      <ToolConsentDialog request={pendingConsent} onDecision={sendConsent} />
    </div>
  );
}
