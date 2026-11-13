import { create } from "zustand";

import type { AssistantSessionContextValue } from "@/components/ai/assistantSession";

/**
 * The assistant session, for chrome that sits outside its provider (the
 * title-bar notification bell). The provider publishes itself while it is
 * mounted, so with AI features off this stays null and nothing offers the
 * assistant. Read it at the user's action (`getState()`), then hand off
 * through `handOffToAssistant`, which captures the book at that moment.
 */
interface AssistantBridgeState {
  assistant: AssistantSessionContextValue | null;
}

export const useAssistantBridge = create<AssistantBridgeState>()(() => ({
  assistant: null,
}));

export function publishAssistantSession(
  assistant: AssistantSessionContextValue | null,
): void {
  useAssistantBridge.setState({ assistant });
}
