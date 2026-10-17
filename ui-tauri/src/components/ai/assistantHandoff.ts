import type {
  AssistantScreenContext,
  AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import { useUiStore } from "@/store/ui";

/**
 * Hand a page's own request to the assistant: the quarantine fix, Source of
 * Funds, chain analysis, PSBT and datasets. It opens the panel and sends the
 * prompt with the screen context of the page that made it (`context`, or the
 * page on screen now), so an "Ask about this" pin never replaces that page's
 * tool pack; the pin is cleared, and the composer chip shows it. Without a
 * chosen model the session parks the prompt in the composer instead, also
 * without the pin, with that page's context and the book it was made in,
 * until it is sent or the draft is cleared or replaced.
 */
export function handOffToAssistant(
  assistant: AssistantSessionContextValue,
  prompt: string,
  context?: AssistantScreenContext,
) {
  useUiStore.getState().openAssistantPanel();
  assistant.sendPrompt(prompt, context ? { context } : {});
}
