/**
 * The open assistant, as the Assistant tab of the right panel: the
 * conversation and its composer, plus the actions the tab bar carries while
 * this tab is in front.
 */

import * as React from "react";
import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { Maximize2, MoreHorizontal, Trash2 } from "lucide-react";

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
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useUiStore } from "@/store/ui";

import { focusAssistantComposer } from "./assistantPanelLayout";
import { useAssistantComposer } from "./useAssistantComposer";

/** Status and conversation actions, shown in the tab bar beside the tabs. */
export function AssistantPanelActions() {
  const { t } = useTranslation("assistant");
  const session = useAssistantSession();
  const hasThread = session.messages.length > 0;
  return (
    <>
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
    </>
  );
}

export function AssistantPanelBody({ inFront }: { inFront: boolean }) {
  const { t } = useTranslation("assistant");
  const session = useAssistantSession();
  const composer = useAssistantComposer();
  const focusRequest = useUiStore((s) => s.assistantPanelFocusRequest);
  const bodyRef = React.useRef<HTMLDivElement>(null);
  const hasThread = session.messages.length > 0;

  // Focus requests come from the shortcut, the launcher, and "Ask about
  // this". The first render also honours one already pending at mount, and
  // one made while the details tab is in front waits for this tab.
  const handledFocusRef = React.useRef(0);
  React.useEffect(() => {
    if (!inFront || focusRequest === handledFocusRef.current) return;
    handledFocusRef.current = focusRequest;
    focusAssistantComposer(bodyRef.current);
  }, [focusRequest, inFront]);

  return (
    <div ref={bodyRef} className="flex min-h-0 flex-1 flex-col">
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
                    useAssistantDraftStore
                      .getState()
                      .replaceDraft(t(`prompts.${key}.prompt`));
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
      <div className="flex shrink-0 flex-col gap-2 p-2">
        {session.error ? (
          <div
            role="alert"
            className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive"
          >
            {session.error.message}
          </div>
        ) : null}
        <AssistantFocusChip />
        {/* A file can be dropped anywhere on the tab, not only on the composer. */}
        <Ai02 {...composer} className="max-w-none" prompts={[]} dropTargetRef={bodyRef} />
      </div>
    </div>
  );
}
