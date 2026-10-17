import { Sparkles } from "lucide-react";
import { useContext } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { AssistantSessionContext } from "@/components/ai/assistantSession";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useUiStore } from "@/store/ui";

interface FixWithAssistantProps {
  /** What needs the user: causes, not every row that follows one. */
  attentionCount: number;
  /** Primary when Kassiber cannot fix anything itself. */
  primary?: boolean;
}

/**
 * Hands what Kassiber cannot decide to the assistant: it gathers the
 * evidence and returns one server-verified plan the owner confirms once.
 * Hidden when the assistant is off.
 */
export function FixWithAssistant({ attentionCount, primary = false }: FixWithAssistantProps) {
  const { t } = useTranslation("journals");
  const { t: tAssistant } = useTranslation("assistant");
  const assistant = useContext(AssistantSessionContext);
  if (!assistant || attentionCount === 0) return null;
  const start = () => {
    if (assistant.isStreaming) return;
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
      variant={primary ? "default" : "outline"}
      onClick={start}
      disabled={assistant.isStreaming}
    >
      <Sparkles className="size-4" aria-hidden="true" />
      {t("quarantine.actions.fixWithAssistant")}
    </Button>
  );
}
