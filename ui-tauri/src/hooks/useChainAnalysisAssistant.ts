import { useContext, useRef } from "react";
import { useTranslation } from "react-i18next";
import { AssistantSessionContext } from "@/components/ai/assistantSession";
import { beginAssistantHandoff } from "@/components/ai/assistantHandoff";
import { DaemonScopeContext, useDaemonMutation } from "@/daemon/client";
import type { AnalysisQuery } from "@/lib/chainAnalysis";

/** The workbench and its public mirror share one scoped, provider-projected handoff. */
export function useChainAnalysisAssistant(onError: (error: unknown) => void, overviewPrompt?: string) {
  const { t } = useTranslation("chainAnalysis");
  const assistant = useContext(AssistantSessionContext);
  const boundary = useContext(DaemonScopeContext);
  const inFlight = useRef(false);
  const projection = useDaemonMutation<{
    query: Record<string, unknown>;
    subject?: string;
    snapshot_id: string;
  }>("ui.chain_analysis.ai_context", { invalidateQueries: false });

  const ask = async (investigation?: { query: AnalysisQuery; snapshot_id: string }, subject?: string) => {
    if (!assistant || assistant.isStreaming || projection.isPending || inFlight.current || boundary?.isCurrent?.() === false) return;
    if (!investigation && !overviewPrompt) return;
    // The page that asked and the book its graph belongs to, captured at the
    // click, before the projection round trip: its tool pack goes with the
    // prompt even if the user navigates meanwhile, and a book opened
    // meanwhile (a switch or a new book) never receives it.
    const handOff = beginAssistantHandoff(assistant, { boundary });
    if (!handOff) return;
    inFlight.current = true;
    try {
      let prompt = overviewPrompt;
      if (investigation) {
        const response = await projection.mutateAsync({
          query: investigation.query,
          expected_snapshot_id: investigation.snapshot_id,
          ...(subject ? { subject } : {}),
        });
        if (!response.data || boundary?.isCurrent?.() === false) return;
        // Only the daemon's projected identities may enter the assistant prompt.
        prompt = t("assistantPrompt", {
          subject: response.data.subject || t("overview"),
          query: JSON.stringify(response.data.query),
          snapshot: response.data.snapshot_id,
        });
      }
      if (!prompt || boundary?.isCurrent?.() === false) return;
      handOff(prompt);
    } catch (error) {
      if (boundary?.isCurrent?.() !== false) onError(error);
    } finally {
      inFlight.current = false;
    }
  };
  return { ask, available: Boolean(assistant), busy: Boolean(assistant?.isStreaming || projection.isPending) };
}
