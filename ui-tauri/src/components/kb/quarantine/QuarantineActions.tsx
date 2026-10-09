import { Sparkles } from "lucide-react";
import { useContext } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { pageHeaderActionClassName } from "@/lib/screen-layout";
import { AssistantSessionContext } from "@/components/ai/assistantSession";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useUiStore } from "@/store/ui";

interface QuarantineActionsProps {
  /** What needs the user: causes, not every row that follows one. */
  attentionCount: number;
}

/**
 * The page's one header action. Recalculating lives with the stale-list
 * notice, and each cause carries its own fix, so neither repeats up here.
 */
export function QuarantineActions({ attentionCount }: QuarantineActionsProps) {
  const { t } = useTranslation("journals");
  const { t: tAssistant } = useTranslation("assistant");
  const assistant = useContext(AssistantSessionContext);
  if (!assistant) return null;
  const investigate = () => {
    if (assistant.isStreaming || attentionCount === 0) return;
    const prompt = tAssistant("review.seedPrompt", { count: attentionCount });
    const state = useUiStore.getState();
    state.setAssistantDockDiscovered(true);
    state.setAssistantDockMinimized(false);
    state.setAssistantDockExpanded(true);
    if (assistant.selection?.model) {
      assistant.sendPrompt(prompt);
    } else {
      useAssistantDraftStore.getState().setDraft(prompt);
    }
  };
  return (
    <Button
      type="button"
      variant="outline"
      className={pageHeaderActionClassName}
      onClick={investigate}
      disabled={attentionCount === 0 || assistant.isStreaming}
      title={attentionCount === 0 ? t("quarantine.actions.investigateEmpty") : undefined}
    >
      <Sparkles className="size-4" aria-hidden="true" />
      {t("quarantine.actions.investigate")}
    </Button>
  );
}
