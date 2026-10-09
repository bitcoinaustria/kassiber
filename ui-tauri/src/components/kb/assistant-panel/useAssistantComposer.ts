import * as React from "react";

import type Ai02 from "@/components/ai-02";
import { useAssistantSession } from "@/components/ai/assistantSession";
import { useSupportedReasoningEffort } from "@/components/ai/useReasoningEffortSupport";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useUiStore } from "@/store/ui";

type ComposerProps = React.ComponentProps<typeof Ai02>;

/**
 * The composer wiring shared by the closed bar and the open panel: both edit
 * the same draft and send through the same session. Sending from anywhere
 * opens the panel so the answer is visible beside the page.
 */
export function useAssistantComposer(): ComposerProps {
  const session = useAssistantSession();
  const draft = useAssistantDraftStore((s) => s.draft);
  const setDraft = useAssistantDraftStore((s) => s.setDraft);
  const supportsThinkingEffort = useSupportedReasoningEffort({
    selection: session.selection,
    thinkingEffort: session.thinkingEffort,
    setThinkingEffort: session.setThinkingEffort,
    enabled: Boolean(session.selection?.provider),
  });
  const { sendPrompt } = session;
  const onSubmit = React.useCallback(
    (prompt: string) => {
      useUiStore.getState().openAssistantPanel();
      sendPrompt(prompt);
    },
    [sendPrompt],
  );
  return {
    selection: session.selection,
    onSelectionChange: session.setSelection,
    value: draft,
    onValueChange: setDraft,
    onSubmit,
    // Held while this book's provider settings seed the effort; the send
    // button says why.
    sendBlockedReason: session.sendBlockedReason ?? null,
    onAbort: session.abort,
    isStreaming: session.isStreaming,
    thinkingEffort: session.thinkingEffort,
    onThinkingEffortChange: supportsThinkingEffort
      ? session.setThinkingEffort
      : undefined,
    showThinkingEffort: supportsThinkingEffort,
    fastMode: session.fastMode,
    onFastModeChange: session.setFastMode,
    modelPickerEnabled: true,
    onAttach: session.attachFile,
    attachedFilename: session.attachment?.filename ?? null,
    onClearAttachment: session.clearAttachment,
    historyMessages: session.messages,
  };
}
