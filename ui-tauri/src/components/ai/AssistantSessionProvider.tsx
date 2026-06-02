import * as React from "react";

import {
  AssistantSessionContext,
  type AssistantReturnPath,
  type AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import {
  type AiChatRequest,
  type AiToolConsentDecision,
  useAiChatStream,
} from "@/daemon/stream";
import { useUiStore } from "@/store/ui";

interface AssistantSessionProviderProps {
  children: React.ReactNode;
  returnPath: AssistantReturnPath;
}

export function AssistantSessionProvider({
  children,
  returnPath,
}: AssistantSessionProviderProps) {
  const selection = useUiStore((state) => state.assistantModelSelection);
  const setSelection = useUiStore(
    (state) => state.setAssistantModelSelection,
  );
  const [thinkingEffort, setThinkingEffort] = React.useState<
    AssistantSessionContextValue["thinkingEffort"]
  >("auto");
  const {
    messages,
    isStreaming,
    send,
    abort,
    error,
    pendingConsent,
    sendConsent,
    reset,
  } = useAiChatStream();
  const [queuedPrompts, setQueuedPrompts] = React.useState<string[]>([]);

  const dispatchPrompt = React.useCallback(
    (prompt: string) => {
      if (!selection?.model) return;
      const userMessages: AiChatRequest["messages"] = messages
        .filter((message) => message.role !== "system")
        .map((message) => ({
          role: message.role,
          content: message.content,
        }));
      const next: AiChatRequest["messages"] = [
        ...userMessages,
        { role: "user", content: prompt },
      ];
      void send(
        {
          provider: selection.provider,
          model: selection.model,
          messages: next,
          options:
            thinkingEffort === "auto"
              ? undefined
              : { reasoning_effort: thinkingEffort },
          toolsEnabled: true,
          toolLoopMaxIterations: 8,
          systemPromptKind: "kassiber",
        },
        prompt,
      );
    },
    [messages, selection, send, thinkingEffort],
  );

  const sendPrompt = React.useCallback(
    (prompt: string) => {
      const trimmed = prompt.trim();
      if (!trimmed || !selection?.model) return;
      if (isStreaming) {
        setQueuedPrompts((current) => [...current, trimmed]);
        return;
      }
      dispatchPrompt(trimmed);
    },
    [dispatchPrompt, isStreaming, selection],
  );

  React.useEffect(() => {
    if (isStreaming || queuedPrompts.length === 0) return;
    if (!selection?.model) return;
    const [nextPrompt] = queuedPrompts;
    setQueuedPrompts((current) => current.slice(1));
    dispatchPrompt(nextPrompt);
  }, [dispatchPrompt, isStreaming, queuedPrompts, selection]);

  const typedSendConsent = React.useCallback(
    (decision: AiToolConsentDecision) => sendConsent(decision),
    [sendConsent],
  );

  const clearChat = React.useCallback(() => {
    setQueuedPrompts([]);
    reset();
  }, [reset]);

  const value = React.useMemo<AssistantSessionContextValue>(
    () => ({
      messages,
      isStreaming,
      error,
      pendingConsent,
      queuedPrompts,
      selection,
      thinkingEffort,
      returnPath,
      setSelection,
      setThinkingEffort,
      sendPrompt,
      sendConsent: typedSendConsent,
      abort,
      reset: clearChat,
    }),
    [
      abort,
      clearChat,
      error,
      isStreaming,
      messages,
      pendingConsent,
      queuedPrompts,
      returnPath,
      setSelection,
      selection,
      sendPrompt,
      thinkingEffort,
      typedSendConsent,
    ],
  );

  return (
    <AssistantSessionContext.Provider value={value}>
      {children}
    </AssistantSessionContext.Provider>
  );
}
