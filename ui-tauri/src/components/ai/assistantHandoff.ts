import type {
  AssistantScreenContext,
  AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useUiStore } from "@/store/ui";

/**
 * Hand a page's own request to the assistant: the quarantine fix, Source of
 * Funds, chain analysis, PSBT and datasets. It opens the panel and sends the
 * prompt with the screen context of the page that made it (`context`, or the
 * page on screen now), so an "Ask about this" pin never replaces that page's
 * tool pack; the pin is cleared, and the composer chip shows it. Without a
 * chosen model the prompt waits in the composer instead, also without the pin.
 */
export function handOffToAssistant(
  assistant: AssistantSessionContextValue,
  prompt: string,
  context?: AssistantScreenContext,
) {
  useUiStore.getState().openAssistantPanel();
  if (assistant.selection?.model) {
    assistant.sendPrompt(prompt, context ? { context } : {});
    return;
  }
  assistant.clearFocus?.();
  useAssistantDraftStore.getState().setDraft(prompt);
}
