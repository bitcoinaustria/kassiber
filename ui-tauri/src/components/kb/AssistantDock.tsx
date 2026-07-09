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
import { useTranslation } from "react-i18next";
import {
  ChevronDown,
  ChevronUp,
  Maximize2,
  MessageSquareText,
  Minus,
  MoreHorizontal,
  Sparkles,
  Trash2,
} from "lucide-react";

import Ai02 from "@/components/ai-02";
import { useAssistantSession } from "@/components/ai/assistantSession";
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
import { cn } from "@/lib/utils";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useUiStore, type AssistantDockPosition } from "@/store/ui";

interface AssistantDockProps {
  className?: string;
  collapsed?: boolean;
  /** macOS-Dock-style auto-hide: park at the bottom edge, reveal on hover. */
  autoHide?: boolean;
  position?: AssistantDockPosition;
}

/** Grace period before the revealed dock parks again after the pointer leaves. */
const AUTO_HIDE_LEAVE_DELAY_MS = 450;

const DOCK_POSITION_CLASS: Record<AssistantDockPosition, string> = {
  left: "mr-auto ml-0",
  center: "mx-auto",
  right: "ml-auto mr-0",
};

export function AssistantDock({
  className,
  collapsed = false,
  autoHide = false,
  position = "center",
}: AssistantDockProps) {
  const { t } = useTranslation("assistant");
  const [isInteracting, setIsInteracting] = React.useState(false);
  const [isThreadCollapsed, setIsThreadCollapsed] = React.useState(false);
  const [isRevealed, setIsRevealed] = React.useState(false);
  const parkTimeoutRef = React.useRef<number | null>(null);
  const assistantDraft = useAssistantDraftStore((s) => s.draft);
  const setAssistantDraft = useAssistantDraftStore((s) => s.setDraft);
  const isMinimized = useUiStore((s) => s.assistantDockMinimized);
  const setIsMinimized = useUiStore((s) => s.setAssistantDockMinimized);
  const setDockExpanded = useUiStore((s) => s.setAssistantDockExpanded);

  const cancelPark = React.useCallback(() => {
    if (parkTimeoutRef.current !== null) {
      window.clearTimeout(parkTimeoutRef.current);
      parkTimeoutRef.current = null;
    }
  }, []);

  const reveal = React.useCallback(() => {
    cancelPark();
    setIsRevealed(true);
  }, [cancelPark]);

  const scheduleParkOnLeave = React.useCallback(() => {
    cancelPark();
    // A composer dropdown (model/reasoning picker) renders through a portal
    // outside this element, so moving into it fires pointerleave here. Don't
    // park while such a menu is open — keep re-checking until it closes, or
    // the dock would slide away and unmount the menu mid-selection.
    const attemptPark = () => {
      if (
        typeof document !== "undefined" &&
        document.querySelector('[role="menu"]')
      ) {
        parkTimeoutRef.current = window.setTimeout(
          attemptPark,
          AUTO_HIDE_LEAVE_DELAY_MS,
        );
        return;
      }
      parkTimeoutRef.current = null;
      setIsRevealed(false);
    };
    parkTimeoutRef.current = window.setTimeout(
      attemptPark,
      AUTO_HIDE_LEAVE_DELAY_MS,
    );
  }, [cancelPark]);

  React.useEffect(() => cancelPark, [cancelPark]);
  const {
    messages,
    isStreaming,
    abort,
    error,
    pendingConsent,
    queuedPrompts,
    sendConsent,
    selection,
    setSelection,
    thinkingEffort,
    setThinkingEffort,
    sendPrompt,
    reset,
  } = useAssistantSession();

  const hasThread = messages.length > 0;
  const queuedPromptCount = queuedPrompts.length;
  // Once a conversation is started the dock stays docked — auto-hide only
  // applies to the idle, thread-less composer.
  const effectiveAutoHide = autoHide && !hasThread;
  // Focus, streaming, or a pending consent dialog transiently pins the dock
  // open even while auto-hide would otherwise park it.
  const pinned =
    isInteracting || isStreaming || Boolean(error) || Boolean(pendingConsent);
  const parked = effectiveAutoHide && !pinned && !isRevealed;
  // A started conversation the user deliberately collapsed to the pill.
  const minimized = hasThread && isMinimized;
  // Both idle-parked (auto-hide) and minimized collapse to the same labeled
  // pill so there is always an unmistakable, discoverable affordance.
  const showHandle = parked || minimized;
  // The peeking compact composer is retired in favour of the pill; `compact`
  // now only drives the legacy scroll-collapse when auto-hide is off.
  const compact = effectiveAutoHide
    ? false
    : collapsed && !hasThread && !isInteracting;
  const showThread = hasThread && !isThreadCollapsed && !minimized;
  const modelPickerEnabled = !compact || hasThread || isStreaming;
  const supportsThinkingEffort = useSupportedReasoningEffort({
    selection,
    thinkingEffort,
    setThinkingEffort,
    enabled: modelPickerEnabled || Boolean(selection?.provider),
  });

  React.useEffect(() => {
    if (!hasThread) {
      setIsThreadCollapsed(false);
      setIsMinimized(false);
    }
  }, [hasThread, setIsMinimized]);

  // Tell the shell whether the dock currently expands over content (a live,
  // non-minimized conversation) so it can reserve real bottom padding.
  React.useEffect(() => {
    setDockExpanded(hasThread && !minimized);
    return () => setDockExpanded(false);
  }, [hasThread, minimized, setDockExpanded]);

  // New activity (a stream, an error, or a tool-consent prompt) always brings
  // a minimized conversation back so the user sees what needs attention.
  React.useEffect(() => {
    if (isMinimized && (isStreaming || Boolean(error) || Boolean(pendingConsent))) {
      setIsMinimized(false);
    }
  }, [isMinimized, isStreaming, error, pendingConsent, setIsMinimized]);

  return (
    <>
      {parked ? (
        // Invisible reveal strip along the bottom edge. Tall enough to catch a
        // move toward the edge without flicker, but still edge-anchored so it
        // isn't triggered by ordinary cursor travel.
        <div
          aria-hidden="true"
          className="pointer-events-auto absolute inset-x-0 bottom-0 z-30 h-6"
          onPointerEnter={reveal}
        />
      ) : null}
      <section
        aria-label={t("dock.label")}
        className={cn(
          "pointer-events-none relative z-30 px-3 pb-3 sm:px-4 sm:pb-4 md:px-6",
          showHandle ? "md:pb-3" : "md:pb-5",
          className,
        )}
      >
        {/*
          A single, always-mounted card is the stable hover target: its content
          swaps between the collapsed pill and the full dock, so pointer
          enter/leave stay reliable and there is no laggy width animation.
        */}
        <div
          className={cn(
            // `relative` keeps the hover buffer (absolute -inset-4) scoped to
            // the card (its only reliable containing block across themes).
            // Flat page language, but floating: `bg-card` lifts it off the
            // `bg-background` page (they're distinct tokens), a full-strength
            // hairline separates it from same-tone content cards, and a
            // layered shadow carries the elevation. No glass/backdrop tricks.
            "pointer-events-auto relative flex border border-border bg-card shadow-[0_24px_70px_rgba(15,23,42,0.18),0_6px_20px_rgba(15,23,42,0.10)] dark:border-white/12 dark:shadow-[0_24px_70px_rgba(0,0,0,0.6),0_4px_16px_rgba(0,0,0,0.4)]",
            DOCK_POSITION_CLASS[position],
            showHandle
              ? "w-fit flex-row items-center rounded-full p-1"
              : cn(
                  // rounded-3xl (24px) − p-2 (8px) = the composer's
                  // rounded-2xl (16px): concentric corners, no uneven lip.
                  "w-full flex-col rounded-3xl p-2",
                  showThread
                    ? "max-w-5xl gap-2"
                    : compact
                      ? "max-w-xl gap-2"
                      : "max-w-3xl gap-3",
                ),
          )}
          onPointerEnter={effectiveAutoHide ? reveal : undefined}
          onPointerLeave={effectiveAutoHide ? scheduleParkOnLeave : undefined}
          onFocusCapture={() => {
            if (effectiveAutoHide) reveal();
            setIsInteracting(true);
          }}
          onBlurCapture={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget)) {
              setIsInteracting(false);
            }
          }}
        >
          {effectiveAutoHide && !showHandle ? (
            // Transparent hover margin around the dock so a near-miss with the
            // cursor doesn't park it. Sits behind the content (the card's
            // backdrop-blur forms a stacking context), so it never intercepts
            // clicks on the composer or its buttons.
            <div
              aria-hidden="true"
              className="pointer-events-auto absolute -inset-4 -z-10"
            />
          ) : null}
          {showHandle ? (
            <button
              type="button"
              className="flex items-center gap-2 rounded-full px-3 py-1.5 text-xs font-medium text-foreground outline-none transition-colors hover:bg-black/5 focus-visible:ring-2 focus-visible:ring-ring dark:hover:bg-white/5"
              onClick={minimized ? () => setIsMinimized(false) : reveal}
              aria-label={minimized ? t("dock.restore") : t("dock.handleHint")}
              title={minimized ? t("dock.restore") : t("dock.handleHint")}
            >
              {/* Both collapsed chips lead with the Sparkles mark so they read
                  as the assistant entry point — deliberately different from the
                  open conversation's message-square header. */}
              <Sparkles
                className="h-4 w-4 text-muted-foreground"
                aria-hidden="true"
              />
              <span>
                {minimized ? t("dock.restore") : t("dock.handleHint")}
              </span>
              {minimized ? (
                <span className="rounded-full bg-muted-foreground/15 px-1.5 py-0.5 text-[10px] font-semibold tabular-nums text-muted-foreground">
                  {messages.length}
                </span>
              ) : null}
              {minimized && isStreaming ? (
                <span className="rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium uppercase text-primary">
                  {queuedPromptCount > 0
                    ? t("page.generatingQueued", { count: queuedPromptCount })
                    : t("page.generating")}
                </span>
              ) : null}
            </button>
          ) : (
            <>
              {showThread ? (
                // Thread sits directly on the card (assistant-page style):
                // no nested box, just a hairline divider under the header.
                <div className="min-h-0 overflow-hidden">
                  <div className="flex items-center gap-2 border-b border-border/40 px-2 pb-2 pt-1 text-xs text-muted-foreground">
                    <MessageSquareText
                      className="ml-1 h-3.5 w-3.5"
                      aria-hidden="true"
                    />
                    <span className="font-medium text-foreground">
                      {t("dock.conversation")}
                    </span>
                    <span>
                      {t("page.messageCount", { count: messages.length })}
                    </span>
                    {isStreaming ? (
                      <span className="ml-1 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium uppercase text-primary">
                        {queuedPromptCount > 0
                          ? t("page.generatingQueued", {
                              count: queuedPromptCount,
                            })
                          : t("page.generating")}
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
                        aria-label={t("dock.openAssistantPage")}
                        title={t("dock.openAssistantPage")}
                      >
                        <Maximize2 className="h-3.5 w-3.5" aria-hidden="true" />
                      </Link>
                    </Button>
                    <Button
                      type="button"
                      variant="ghost"
                      size="icon-xs"
                      className="rounded-full"
                      onClick={() => setIsMinimized(true)}
                      aria-label={t("dock.minimize")}
                      title={t("dock.minimize")}
                    >
                      <Minus className="h-3.5 w-3.5" aria-hidden="true" />
                    </Button>
                    {/* Rare actions live behind "…", mirroring the page toolbar. */}
                    <DropdownMenu>
                      <DropdownMenuTrigger asChild>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon-xs"
                          className="rounded-full"
                          aria-label={t("page.moreActions")}
                          title={t("page.moreActions")}
                        >
                          <MoreHorizontal
                            className="h-3.5 w-3.5"
                            aria-hidden="true"
                          />
                        </Button>
                      </DropdownMenuTrigger>
                      <DropdownMenuContent align="end" className="min-w-52">
                        <DropdownMenuItem
                          onSelect={() => setIsThreadCollapsed(true)}
                        >
                          <ChevronDown className="size-4" aria-hidden="true" />
                          {t("dock.collapseConversation")}
                        </DropdownMenuItem>
                        <DropdownMenuSeparator />
                        <DropdownMenuItem
                          variant="destructive"
                          onSelect={reset}
                        >
                          <Trash2 className="size-4" aria-hidden="true" />
                          {t("page.clearChat")}
                        </DropdownMenuItem>
                      </DropdownMenuContent>
                    </DropdownMenu>
                  </div>
                  <ChatThread
                    messages={messages}
                    className="max-h-[min(52vh,520px)] max-w-none"
                    contentClassName="px-3 py-3"
                  />
                </div>
              ) : null}
              {hasThread && isThreadCollapsed ? (
                <div className="flex w-full items-center gap-1.5 rounded-2xl bg-muted/60 px-2 py-1.5 text-xs text-muted-foreground">
                  <button
                    type="button"
                    className="flex min-w-0 flex-1 items-center gap-2 rounded-xl px-1 py-1 text-left transition-colors hover:bg-muted/60"
                    onClick={() => setIsThreadCollapsed(false)}
                    aria-label={t("dock.expandConversation")}
                  >
                    <MessageSquareText
                      className="h-3.5 w-3.5 shrink-0"
                      aria-hidden="true"
                    />
                    <span className="font-medium text-foreground">
                      {t("dock.conversationHidden")}
                    </span>
                    <span className="min-w-0 flex-1 truncate">
                      {t("dock.preservedForContext", {
                        count: messages.length,
                      })}
                    </span>
                    {isStreaming ? (
                      <span className="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium uppercase text-primary">
                        {queuedPromptCount > 0
                          ? t("page.generatingQueued", {
                              count: queuedPromptCount,
                            })
                          : t("page.generating")}
                      </span>
                    ) : null}
                    <ChevronUp
                      className="h-3.5 w-3.5 shrink-0"
                      aria-hidden="true"
                    />
                  </button>
                  <Button
                    asChild
                    variant="ghost"
                    size="icon-xs"
                    className="rounded-full"
                  >
                    <Link
                      to="/assistant"
                      aria-label={t("dock.openAssistantPage")}
                      title={t("dock.openAssistantPage")}
                    >
                      <Maximize2 className="h-3.5 w-3.5" aria-hidden="true" />
                    </Link>
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-xs"
                    className="rounded-full"
                    onClick={() => setIsMinimized(true)}
                    aria-label={t("dock.minimize")}
                    title={t("dock.minimize")}
                  >
                    <Minus className="h-3.5 w-3.5" aria-hidden="true" />
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
                // Same borderless bg-muted input box as the assistant page.
                composerClassName="border-0 bg-muted shadow-none backdrop-blur-0 dark:bg-muted"
                compact={compact}
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
                inputPanelElevated={false}
                modelPickerEnabled={
                  modelPickerEnabled || Boolean(selection?.provider)
                }
                // Page parity: once a conversation exists the quick-prompt
                // chips disappear — they only help start a chat.
                {...(hasThread ? { prompts: [] } : {})}
              />
            </>
          )}
          <ToolConsentDialog request={pendingConsent} onDecision={sendConsent} />
        </div>
      </section>
    </>
  );
}
