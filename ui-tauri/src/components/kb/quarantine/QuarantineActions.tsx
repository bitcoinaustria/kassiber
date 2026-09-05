import { Link } from "@tanstack/react-router";
import { ListChecks, Loader2, RefreshCw, Sparkles, TableProperties } from "lucide-react";
import { useContext } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { pageHeaderActionClassName } from "@/lib/screen-layout";
import { AssistantSessionContext } from "@/components/ai/assistantSession";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useUiStore } from "@/store/ui";

interface QuarantineActionsProps {
  isProcessingJournals: boolean;
  onProcessJournals: () => void;
  onOpenResolvePlan: () => void;
  resolvePlanCount: number;
  quarantineCount: number;
}

export function QuarantineActions({
  isProcessingJournals,
  onProcessJournals,
  onOpenResolvePlan,
  resolvePlanCount,
  quarantineCount,
}: QuarantineActionsProps) {
  const { t } = useTranslation("journals");
  const { t: tAssistant } = useTranslation("assistant");
  const assistant = useContext(AssistantSessionContext);
  const investigate = () => {
    if (!assistant || assistant.isStreaming || quarantineCount === 0) return;
    const prompt = tAssistant("review.seedPrompt", { count: quarantineCount });
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
    <>
      {assistant ? <Button type="button" className={pageHeaderActionClassName}
        onClick={investigate} disabled={quarantineCount === 0 || assistant.isStreaming}
        title={quarantineCount === 0 ? t("quarantine.actions.investigateEmpty") : undefined}>
        <Sparkles className="size-4" aria-hidden="true" />{t("quarantine.actions.investigate")}
      </Button> : null}
      <Button
        type="button"
        variant="outline"
        className={pageHeaderActionClassName}
        onClick={onOpenResolvePlan}
        disabled={resolvePlanCount === 0}
      >
        <ListChecks className="size-4" aria-hidden="true" />
        {t("quarantine.resolvePlan.button")}
      </Button>
      <Button asChild variant="outline" className={pageHeaderActionClassName}>
        <Link to="/transactions">
          <TableProperties className="size-4" aria-hidden="true" />
          {t("quarantine.actions.transactions")}
        </Link>
      </Button>
      <Button
        type="button"
        variant="outline"
        className={pageHeaderActionClassName}
        onClick={onProcessJournals}
        disabled={isProcessingJournals}
      >
        {isProcessingJournals ? (
          <Loader2 className="size-4 animate-spin" aria-hidden="true" />
        ) : (
          <RefreshCw className="size-4" aria-hidden="true" />
        )}
        {t("quarantine.actions.processJournals")}
      </Button>
    </>
  );
}
