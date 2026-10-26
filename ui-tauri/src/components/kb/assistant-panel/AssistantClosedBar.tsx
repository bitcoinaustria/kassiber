/**
 * What stays below the page while the assistant panel is closed. It is a
 * normal flex child under the page's scroll area, so it takes its own height
 * instead of floating over the content.
 */

import { useTranslation } from "react-i18next";
import { MessageSquareText, Sparkles } from "lucide-react";

import Ai02 from "@/components/ai-02";
import { useAssistantSession } from "@/components/ai/assistantSession";
import { Kbd } from "@/components/ui/kbd";
import { formatShortcut } from "@/lib/shortcutLabel";
import { cn } from "@/lib/utils";
import {
  useUiStore,
  type AssistantClosedBarMode,
  type AssistantDockPosition,
} from "@/store/ui";

import { ASSISTANT_PANEL_SHORTCUT } from "./assistantPanelLayout";
import { useAssistantComposer } from "./useAssistantComposer";

const POSITION_CLASS: Record<AssistantDockPosition, string> = {
  left: "mr-auto",
  center: "mx-auto",
  right: "ml-auto",
};

export function AssistantClosedBar({
  mode,
  position,
}: {
  mode: Exclude<AssistantClosedBarMode, "hidden">;
  position: AssistantDockPosition;
}) {
  const { t } = useTranslation("assistant");
  const session = useAssistantSession();
  const openPanel = useUiStore((s) => s.openAssistantPanel);
  const shortcut = formatShortcut([...ASSISTANT_PANEL_SHORTCUT]);
  const messageCount = session.messages.length;

  const conversationButton =
    messageCount > 0 ? (
      <button
        type="button"
        className="inline-flex shrink-0 items-center gap-1.5 rounded-full px-2.5 py-1.5 text-xs font-medium text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
        onClick={() => openPanel({ focusComposer: true })}
        aria-label={t("panel.openConversation")}
        title={`${t("panel.openConversation")} (${shortcut})`}
      >
        {session.isStreaming ? (
          <span
            className="size-1.5 animate-pulse rounded-full bg-primary"
            aria-hidden="true"
          />
        ) : (
          <MessageSquareText className="size-3.5" aria-hidden="true" />
        )}
        {session.isStreaming
          ? t("dock.working")
          : t("page.messageCount", { count: messageCount })}
      </button>
    ) : null;

  return (
    <section
      aria-label={t("dock.label")}
      data-assistant-closed-bar={mode}
      className="shrink-0 border-t border-border/50 bg-background px-3 py-1.5 sm:px-4"
    >
      {mode === "launcher" ? (
        <div className={cn("flex w-fit items-center gap-1", POSITION_CLASS[position])}>
          {conversationButton}
          <button
            type="button"
            className="group inline-flex items-center gap-2 rounded-full px-2.5 py-1.5 text-xs font-medium text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            onClick={() => openPanel({ focusComposer: true })}
            aria-label={t("panel.open")}
          >
            <Sparkles className="size-3.5 text-primary" aria-hidden="true" />
            {t("panel.launcher")}
            <Kbd>{shortcut}</Kbd>
          </button>
        </div>
      ) : (
        <div
          className={cn(
            "flex w-full max-w-xl items-end gap-1",
            POSITION_CLASS[position],
          )}
        >
          {conversationButton ? (
            <div className="flex h-11 items-center">{conversationButton}</div>
          ) : null}
          <ClosedComposer />
        </div>
      )}
    </section>
  );
}

function ClosedComposer() {
  const composer = useAssistantComposer();
  return (
    <Ai02
      {...composer}
      className="min-w-0 max-w-none flex-1"
      compact
      prompts={[]}
    />
  );
}
