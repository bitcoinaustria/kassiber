/**
 * The open assistant: a right-side panel beside the page (the page column
 * shrinks; nothing is covered), or an overlay sheet on narrow windows.
 * Resizable from its left edge; the width is remembered in the UI store.
 */

import * as React from "react";
import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import {
  Maximize2,
  MoreHorizontal,
  PanelRightClose,
  Sparkles,
  Trash2,
} from "lucide-react";

import Ai02 from "@/components/ai-02";
import { AssistantFocusChip } from "@/components/ai/AssistantFocusChip";
import { useAssistantSession } from "@/components/ai/assistantSession";
import { ChatThread } from "@/components/ai/ChatThread";
import { DEFAULT_COMPOSER_PROMPT_KEYS } from "@/components/ai/composerPrompts";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { formatShortcut } from "@/lib/shortcutLabel";
import { cn } from "@/lib/utils";
import { ASSISTANT_PANEL_MIN_WIDTH, useUiStore } from "@/store/ui";

import {
  ASSISTANT_PANEL_SHORTCUT,
  ASSISTANT_PANEL_MIN_PAGE_WIDTH,
  assistantPanelMaxWidth,
  clampAssistantPanelWidth,
  draggedAssistantPanelWidth,
  keyedAssistantPanelWidth,
} from "./assistantPanelLayout";
import { useAssistantComposer } from "./useAssistantComposer";

function clearDragStyles() {
  document.body.style.removeProperty("cursor");
  document.body.style.removeProperty("user-select");
}

export function AssistantSidePanel({ overlay }: { overlay: boolean }) {
  const { t } = useTranslation("assistant");
  const session = useAssistantSession();
  const composer = useAssistantComposer();
  const storedWidth = useUiStore((s) => s.assistantPanelWidth);
  const setStoredWidth = useUiStore((s) => s.setAssistantPanelWidth);
  const setOpen = useUiStore((s) => s.setAssistantPanelOpen);
  const focusRequest = useUiStore((s) => s.assistantPanelFocusRequest);
  const panelRef = React.useRef<HTMLElement>(null);
  const [dragWidth, setDragWidth] = React.useState<number | null>(null);
  const dragRef = React.useRef<{ startX: number; startWidth: number } | null>(
    null,
  );
  const close = React.useCallback(() => setOpen(false), [setOpen]);
  const hasThread = session.messages.length > 0;
  const width = dragWidth ?? storedWidth;
  const containerWidth = () =>
    panelRef.current?.parentElement?.clientWidth || null;
  // CSS keeps the page its minimum, so a width remembered on a wider window
  // can render narrower than stored. Resizing starts from what is on screen,
  // or every step until the stored width caught up would change nothing.
  const renderedWidth = () => {
    const measured = panelRef.current?.getBoundingClientRect().width ?? 0;
    return clampAssistantPanelWidth(measured > 0 ? measured : width, containerWidth());
  };

  // Focus requests come from the shortcut, the launcher, and "Ask about
  // this". The first render also honours one already pending at mount.
  const handledFocusRef = React.useRef(0);
  React.useEffect(() => {
    if (focusRequest === handledFocusRef.current) return;
    handledFocusRef.current = focusRequest;
    const field = panelRef.current?.querySelector<HTMLTextAreaElement>(
      "textarea",
    );
    if (!field) return;
    field.focus();
    const end = field.value.length;
    field.setSelectionRange(end, end);
  }, [focusRequest]);

  const finishDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag) return;
    dragRef.current = null;
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    clearDragStyles();
    // A click on the handle is not a resize; keep the remembered width.
    if (dragWidth !== null && dragWidth !== drag.startWidth) setStoredWidth(dragWidth);
    setDragWidth(null);
  };

  // A drag can end without a pointer-up: the panel closes (shortcut) or turns
  // into an overlay, and the handle goes with it. Drop that drag rather than
  // leaving the page stuck with the resize cursor and no text selection.
  React.useEffect(() => {
    if (!overlay) return undefined;
    if (dragRef.current) {
      dragRef.current = null;
      clearDragStyles();
      setDragWidth(null);
    }
    return undefined;
  }, [overlay]);
  React.useEffect(
    () => () => {
      if (dragRef.current) {
        dragRef.current = null;
        clearDragStyles();
      }
    },
    [],
  );

  return (
    <>
      {overlay ? (
        <button
          type="button"
          aria-label={t("panel.close")}
          tabIndex={-1}
          className="absolute inset-0 z-30 bg-black/30 motion-safe:animate-in motion-safe:fade-in"
          onClick={close}
        />
      ) : null}
      <aside
        ref={panelRef}
        aria-label={t("dock.label")}
        data-assistant-panel={overlay ? "overlay" : "docked"}
        style={{
          width: overlay ? `min(${width}px, 100%)` : width,
          minWidth: overlay ? undefined : ASSISTANT_PANEL_MIN_WIDTH,
          // A width remembered on a wider window must not squeeze the page
          // below its minimum on this one.
          maxWidth: overlay
            ? undefined
            : `max(${ASSISTANT_PANEL_MIN_WIDTH}px, calc(100% - ${ASSISTANT_PANEL_MIN_PAGE_WIDTH}px))`,
        }}
        className={cn(
          "flex h-full min-h-0 shrink-0 flex-col bg-background",
          overlay
            ? "absolute inset-y-0 right-0 z-40 border-l border-border/70 shadow-2xl motion-safe:animate-in motion-safe:slide-in-from-right-8"
            : "relative border-l border-border/70",
        )}
        onKeyDown={(event) => {
          if (
            overlay &&
            event.key === "Escape" &&
            !document.querySelector('[role="menu"], [role="dialog"]')
          ) {
            event.stopPropagation();
            close();
          }
        }}
      >
        {overlay ? null : (
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label={t("panel.resize")}
            aria-valuemin={ASSISTANT_PANEL_MIN_WIDTH}
            aria-valuemax={assistantPanelMaxWidth(containerWidth())}
            aria-valuenow={clampAssistantPanelWidth(width, containerWidth())}
            tabIndex={0}
            className={cn(
              "group absolute inset-y-0 -left-1.5 z-20 w-3 cursor-col-resize touch-none outline-none",
              "after:absolute after:inset-y-0 after:left-1/2 after:w-px after:-translate-x-1/2 after:transition-colors",
              "hover:after:bg-primary/60 focus-visible:after:bg-primary",
              dragWidth !== null && "after:bg-primary",
            )}
            onPointerDown={(event) => {
              if (event.button !== 0) return;
              event.preventDefault();
              event.currentTarget.setPointerCapture?.(event.pointerId);
              const startWidth = renderedWidth();
              dragRef.current = { startX: event.clientX, startWidth };
              document.body.style.cursor = "col-resize";
              document.body.style.userSelect = "none";
              setDragWidth(startWidth);
            }}
            onPointerMove={(event) => {
              const drag = dragRef.current;
              if (!drag) return;
              setDragWidth(
                draggedAssistantPanelWidth({
                  ...drag,
                  clientX: event.clientX,
                  containerWidth: containerWidth(),
                }),
              );
            }}
            onPointerUp={finishDrag}
            onPointerCancel={finishDrag}
            onDoubleClick={() => {
              const next = keyedAssistantPanelWidth(
                "Enter",
                renderedWidth(),
                containerWidth(),
              );
              if (next !== null) setStoredWidth(next);
            }}
            onKeyDown={(event) => {
              const next = keyedAssistantPanelWidth(
                event.key,
                renderedWidth(),
                containerWidth(),
              );
              if (next === null) return;
              event.preventDefault();
              setStoredWidth(next);
            }}
          />
        )}
        <header className="flex h-11 shrink-0 items-center gap-1.5 border-b border-border/60 px-3 text-xs">
          <Sparkles className="size-3.5 text-primary" aria-hidden="true" />
          <h2 className="text-sm font-medium text-foreground">
            {t("panel.title")}
          </h2>
          {hasThread ? (
            <span className="truncate text-muted-foreground">
              {t("page.messageCount", { count: session.messages.length })}
            </span>
          ) : null}
          {session.isStreaming ? (
            <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-primary/10 px-1.5 py-0.5 text-2xs font-medium text-primary">
              <span
                className="size-1.5 animate-pulse rounded-full bg-primary"
                aria-hidden="true"
              />
              {session.queuedPrompts.length > 0
                ? t("dock.workingQueued", { count: session.queuedPrompts.length })
                : t("dock.working")}
            </span>
          ) : null}
          <div className="ml-auto flex shrink-0 items-center gap-0.5">
            {hasThread ? (
              <DropdownMenu>
                <DropdownMenuTrigger asChild>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon-xs"
                    aria-label={t("page.moreActions")}
                    title={t("page.moreActions")}
                  >
                    <MoreHorizontal className="size-3.5" aria-hidden="true" />
                  </Button>
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="min-w-44">
                  <DropdownMenuItem variant="destructive" onSelect={session.reset}>
                    <Trash2 className="size-4" aria-hidden="true" />
                    {t("page.clearChat")}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            ) : null}
            <Button asChild variant="ghost" size="icon-xs">
              <Link
                to="/assistant"
                aria-label={t("dock.openAssistantPage")}
                title={t("dock.openAssistantPage")}
              >
                <Maximize2 className="size-3.5" aria-hidden="true" />
              </Link>
            </Button>
            <Button
              type="button"
              variant="ghost"
              size="icon-xs"
              onClick={close}
              aria-label={t("panel.close")}
              title={`${t("panel.close")} (${formatShortcut([...ASSISTANT_PANEL_SHORTCUT])})`}
            >
              <PanelRightClose className="size-3.5" aria-hidden="true" />
            </Button>
          </div>
        </header>
        {hasThread ? (
          <ChatThread
            messages={session.messages}
            className="min-h-0 flex-1"
            contentClassName="px-3 py-3"
            onBranchMessage={session.branchFromMessage}
          />
        ) : (
          <div className="flex min-h-0 flex-1 flex-col justify-end gap-1.5 overflow-y-auto px-4 pb-4">
            <p className="text-sm font-medium text-foreground">
              {t("panel.emptyTitle")}
            </p>
            <p className="text-xs leading-5 text-muted-foreground">
              {t("panel.emptyBody")}
            </p>
            {/* A vertical list fits the panel's width; the composer's own
                chip row would wrap and clip here. */}
            <ul className="mt-2 flex flex-col gap-1">
              {DEFAULT_COMPOSER_PROMPT_KEYS.map(({ icon: Icon, key }) => (
                <li key={key}>
                  <button
                    type="button"
                    className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                    onClick={() => {
                      composer.onValueChange?.(t(`prompts.${key}.prompt`));
                      useUiStore.getState().openAssistantPanel({ focusComposer: true });
                    }}
                  >
                    <Icon className="size-4 shrink-0" aria-hidden="true" />
                    {t(`prompts.${key}.text`)}
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
        <div className="flex shrink-0 flex-col gap-2 border-t border-border/40 p-2">
          {session.error ? (
            <div
              role="alert"
              className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive"
            >
              {session.error.message}
            </div>
          ) : null}
          <AssistantFocusChip />
          <Ai02 {...composer} className="max-w-none" prompts={[]} />
        </div>
      </aside>
    </>
  );
}
