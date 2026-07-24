/**
 * Shell-wide assistant footer.
 *
 * State model (Apple-style: discoverable when new, out of the way once known):
 * - Composer peek: first-run idle — looks like the real input, always visible
 * - Parked handle: after discovery — thin edge capsule, reveal on approach
 * - Minimized chip: mid-chat collapse — corner count only
 * - Working + follow-up: streaming while minimized — status chip + slim queue composer
 * - Expanded: full thread + composer
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
  Square,
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

// Understated frosted panel — the same material as the composer's
// `.kb-composer-glass` (hairline border + soft short-throw shadow + a light
// `bg-card` frost), so the floating assistant reads as the ported T3Code chat
// in every state. Replaces the earlier heavy drop shadow + glossy top edge.
const CARD_SURFACE =
  "pointer-events-auto relative flex border border-border/70 bg-card/95 shadow-[0_20px_48px_-24px_rgba(15,23,42,0.45)] backdrop-blur-md dark:border-white/10 dark:bg-card/90 dark:shadow-[0_20px_48px_-22px_rgba(0,0,0,0.65)] origin-bottom transition-[transform,opacity,box-shadow] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none";

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
  const dockDiscovered = useUiStore((s) => s.assistantDockDiscovered);
  const setDockDiscovered = useUiStore((s) => s.setAssistantDockDiscovered);

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
    branchFromMessage,
  } = useAssistantSession();

  const hasThread = messages.length > 0;
  const queuedPromptCount = queuedPrompts.length;

  const markDiscovered = React.useCallback(() => {
    if (!dockDiscovered) setDockDiscovered(true);
  }, [dockDiscovered, setDockDiscovered]);

  const handleSubmit = React.useCallback(
    (prompt: string) => {
      markDiscovered();
      sendPrompt(prompt);
    },
    [markDiscovered, sendPrompt],
  );

  // First-run: always show a composer peek (no park-to-pill). After the first
  // send, auto-hide may park to a thin edge handle.
  const showComposerPeek = !hasThread && !dockDiscovered;
  const effectiveAutoHide = autoHide && !hasThread && dockDiscovered;

  // Focus / consent / error pin the idle dock open; streaming alone does not
  // — minimized chats stay collapsed with a Working chip instead.
  const pinned =
    isInteracting || Boolean(error) || Boolean(pendingConsent);
  const parked = effectiveAutoHide && !pinned && !isRevealed;
  const minimized = hasThread && isMinimized;
  const showWorkingSurface = minimized && isStreaming;
  const showMinimizedChip = minimized && !isStreaming;
  const showCollapsedChrome =
    parked || showMinimizedChip || showWorkingSurface;

  const compact = effectiveAutoHide
    ? false
    : collapsed && !hasThread && !isInteracting && dockDiscovered;
  const showThread = hasThread && !isThreadCollapsed && !minimized;
  const modelPickerEnabled =
    !compact || hasThread || isStreaming || showComposerPeek;
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

  // Reserve real bottom padding for peek, working follow-up, or expanded thread.
  React.useEffect(() => {
    const needsSpace =
      showComposerPeek ||
      showWorkingSurface ||
      (hasThread && !minimized);
    setDockExpanded(needsSpace);
    return () => setDockExpanded(false);
  }, [
    hasThread,
    minimized,
    setDockExpanded,
    showComposerPeek,
    showWorkingSurface,
  ]);

  // Consent / errors still demand attention; streaming no longer forces expand.
  React.useEffect(() => {
    if (isMinimized && (Boolean(error) || Boolean(pendingConsent))) {
      setIsMinimized(false);
    }
  }, [isMinimized, error, pendingConsent, setIsMinimized]);

  const restore = React.useCallback(() => {
    setIsMinimized(false);
    reveal();
  }, [reveal, setIsMinimized]);

  const workingLabel =
    queuedPromptCount > 0
      ? t("dock.workingQueued", { count: queuedPromptCount })
      : t("dock.working");

  return (
    <>
      {parked ? (
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
          showCollapsedChrome ? "md:pb-3" : "md:pb-5",
          className,
        )}
      >
        {showWorkingSurface ? (
          <WorkingFollowUpSurface
            className={CARD_SURFACE}
            workingLabel={workingLabel}
            onRestore={restore}
            onAbort={abort}
            selection={selection}
            onSelectionChange={setSelection}
            value={assistantDraft}
            onValueChange={setAssistantDraft}
            onSubmit={handleSubmit}
            isStreaming={isStreaming}
            thinkingEffort={thinkingEffort}
            onThinkingEffortChange={
              supportsThinkingEffort ? setThinkingEffort : undefined
            }
            showThinkingEffort={supportsThinkingEffort}
            modelPickerEnabled={
              modelPickerEnabled || Boolean(selection?.provider)
            }
            followUpPlaceholder={t("composer.followUpPlaceholder")}
            tStop={t("composer.stopGenerating")}
            tRestore={t("dock.restore")}
          />
        ) : showMinimizedChip ? (
          <MinimizedChip
            className={CARD_SURFACE}
            messageCount={messages.length}
            onRestore={restore}
            label={t("dock.restore")}
            countLabel={t("page.messageCount", { count: messages.length })}
          />
        ) : parked ? (
          <ParkedHandle
            className={CARD_SURFACE}
            onReveal={reveal}
            label={t("dock.handleHint")}
          />
        ) : (
          <div
            className={cn(
              CARD_SURFACE,
              DOCK_POSITION_CLASS[position],
              "w-full flex-col rounded-3xl p-2",
              showThread
                ? "max-w-5xl gap-2"
                : showComposerPeek || compact
                  ? "max-w-xl gap-2"
                  : "max-w-3xl gap-3",
            )}
            onPointerEnter={effectiveAutoHide ? reveal : undefined}
            onPointerLeave={
              effectiveAutoHide ? scheduleParkOnLeave : undefined
            }
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
            {effectiveAutoHide ? (
              <div
                aria-hidden="true"
                className="pointer-events-auto absolute -inset-4 -z-10"
              />
            ) : null}
            <div
              className={cn(
                "flex w-full flex-col motion-safe:animate-in motion-safe:fade-in motion-safe:slide-in-from-bottom-3 motion-safe:duration-300",
                showThread || compact || showComposerPeek ? "gap-2" : "gap-3",
              )}
            >
              {showThread ? (
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
                      <span className="ml-1 inline-flex items-center gap-1 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary">
                        <span
                          className="size-1.5 animate-pulse rounded-full bg-primary"
                          aria-hidden="true"
                        />
                        {workingLabel}
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
                    onBranchMessage={branchFromMessage}
                  />
                </div>
              ) : null}
              {hasThread && isThreadCollapsed ? (
                <div className="flex w-full items-center gap-1.5 rounded-2xl bg-muted/60 px-2 py-1.5 text-xs text-muted-foreground">
                  <button
                    type="button"
                    className="flex min-w-0 flex-1 items-center gap-2 rounded-xl px-1 py-1 text-left transition-all duration-100 ease-out hover:bg-muted/60 active:scale-[0.99]"
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
                      <span className="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-primary">
                        {workingLabel}
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
                composerClassName="border-0 bg-muted shadow-none backdrop-blur-0 dark:bg-muted"
                compact={compact || showComposerPeek}
                selection={selection}
                onSelectionChange={setSelection}
                value={assistantDraft}
                onValueChange={setAssistantDraft}
                onSubmit={handleSubmit}
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
                {...(hasThread || showComposerPeek ? { prompts: [] } : {})}
              />
            </div>
            <ToolConsentDialog
              request={pendingConsent}
              onDecision={sendConsent}
            />
          </div>
        )}
        {showCollapsedChrome ? (
          <ToolConsentDialog
            request={pendingConsent}
            onDecision={sendConsent}
          />
        ) : null}
      </section>
    </>
  );
}

function ParkedHandle({
  className,
  onReveal,
  label,
}: {
  className: string;
  onReveal: () => void;
  label: string;
}) {
  return (
    <div
      className={cn(
        className,
        "mx-auto w-fit flex-row items-center rounded-full p-1",
      )}
    >
      <button
        type="button"
        className="group flex items-center gap-2 rounded-full px-3 py-1.5 text-xs font-medium text-foreground outline-none transition-all duration-100 ease-out hover:bg-black/5 focus-visible:ring-2 focus-visible:ring-ring active:scale-[0.97] dark:hover:bg-white/5"
        onClick={onReveal}
        aria-label={label}
        title={label}
      >
        <span
          className="relative flex size-4 items-center justify-center"
          aria-hidden="true"
        >
          <span className="absolute inset-0 rounded-full bg-primary/20 opacity-70 blur-[2px] transition-opacity group-hover:opacity-100" />
          <Sparkles className="relative h-3.5 w-3.5 text-primary" />
        </span>
        <span className="text-muted-foreground group-hover:text-foreground">
          {label}
        </span>
      </button>
    </div>
  );
}

function MinimizedChip({
  className,
  messageCount,
  onRestore,
  label,
  countLabel,
}: {
  className: string;
  messageCount: number;
  onRestore: () => void;
  label: string;
  countLabel: string;
}) {
  return (
    <div
      className={cn(
        className,
        "ml-auto w-fit flex-row items-center rounded-full p-1",
      )}
    >
      <button
        type="button"
        className="flex items-center gap-2 rounded-full px-2.5 py-1.5 text-xs font-medium text-foreground outline-none transition-all duration-100 ease-out hover:bg-black/5 focus-visible:ring-2 focus-visible:ring-ring active:scale-[0.97] dark:hover:bg-white/5"
        onClick={onRestore}
        aria-label={label}
        title={`${label} · ${countLabel}`}
      >
        <Sparkles
          className="h-3.5 w-3.5 text-muted-foreground"
          aria-hidden="true"
        />
        <span className="rounded-full bg-muted-foreground/15 px-1.5 py-0.5 text-[10px] font-semibold tabular-nums text-muted-foreground">
          {messageCount}
        </span>
      </button>
    </div>
  );
}

function WorkingFollowUpSurface({
  className,
  workingLabel,
  onRestore,
  onAbort,
  selection,
  onSelectionChange,
  value,
  onValueChange,
  onSubmit,
  isStreaming,
  thinkingEffort,
  onThinkingEffortChange,
  showThinkingEffort,
  modelPickerEnabled,
  followUpPlaceholder,
  tStop,
  tRestore,
}: {
  className: string;
  workingLabel: string;
  onRestore: () => void;
  onAbort?: () => void;
  selection: { provider: string; model: string } | null;
  onSelectionChange: (next: { provider: string; model: string } | null) => void;
  value: string;
  onValueChange: (value: string) => void;
  onSubmit: (prompt: string) => void;
  isStreaming: boolean;
  thinkingEffort: "auto" | "low" | "medium" | "high";
  onThinkingEffortChange?: (effort: "auto" | "low" | "medium" | "high") => void;
  showThinkingEffort: boolean;
  modelPickerEnabled: boolean;
  followUpPlaceholder: string;
  tStop: string;
  tRestore: string;
}) {
  return (
    <div
      className={cn(
        className,
        "ml-auto w-full max-w-md flex-col gap-2 rounded-3xl p-2 motion-safe:animate-in motion-safe:fade-in motion-safe:slide-in-from-bottom-3 motion-safe:duration-300",
      )}
    >
      <div className="flex items-center gap-1.5 px-1">
        <button
          type="button"
          className="inline-flex min-w-0 flex-1 items-center gap-2 rounded-full px-2 py-1 text-left text-xs font-medium text-foreground outline-none transition-all duration-100 ease-out hover:bg-black/5 focus-visible:ring-2 focus-visible:ring-ring active:scale-[0.99] dark:hover:bg-white/5"
          onClick={onRestore}
          aria-label={tRestore}
          title={tRestore}
        >
          <span
            className="relative flex size-4 shrink-0 items-center justify-center"
            aria-hidden="true"
          >
            <span className="absolute inset-0 animate-pulse rounded-full bg-primary/25" />
            <Sparkles className="relative h-3.5 w-3.5 text-primary" />
          </span>
          <span className="truncate text-muted-foreground">{workingLabel}</span>
        </button>
        {onAbort ? (
          <Button
            type="button"
            variant="ghost"
            size="icon-xs"
            className="rounded-full"
            onClick={onAbort}
            aria-label={tStop}
            title={tStop}
          >
            <Square className="h-3 w-3" aria-hidden="true" />
          </Button>
        ) : null}
      </div>
      <Ai02
        className="max-w-none border-0 bg-transparent p-0 shadow-none ring-0 backdrop-blur-0"
        composerClassName="border-0 bg-muted shadow-none backdrop-blur-0 dark:bg-muted"
        compact
        selection={selection}
        onSelectionChange={onSelectionChange}
        value={value}
        onValueChange={onValueChange}
        onSubmit={onSubmit}
        onAbort={onAbort}
        isStreaming={isStreaming}
        thinkingEffort={thinkingEffort}
        onThinkingEffortChange={onThinkingEffortChange}
        showThinkingEffort={showThinkingEffort}
        inputPanelElevated={false}
        modelPickerEnabled={modelPickerEnabled}
        placeholder={followUpPlaceholder}
        prompts={[]}
      />
    </div>
  );
}
