import * as React from "react";

import {
  AssistantSessionContext,
  type AssistantModelSelection,
  type AssistantReturnPath,
  type AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import {
  type AiChatRequest,
  type AiToolConsentDecision,
  useAiChatStream,
} from "@/daemon/stream";

interface AssistantSessionProviderProps {
  children: React.ReactNode;
  returnPath: AssistantReturnPath;
}

export function AssistantSessionProvider({
  children,
  returnPath,
}: AssistantSessionProviderProps) {
  const [selection, setSelection] =
    React.useState<AssistantModelSelection | null>(null);
  const [toolsEnabled, setToolsEnabled] = React.useState(true);
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

  const sendPrompt = React.useCallback(
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
          toolsEnabled,
          toolLoopMaxIterations: 8,
          systemPromptKind: toolsEnabled ? "kassiber" : null,
        },
        prompt,
      );
    },
    [messages, selection, send, toolsEnabled],
  );

  const typedSendConsent = React.useCallback(
    (decision: AiToolConsentDecision) => sendConsent(decision),
    [sendConsent],
  );

  const value = React.useMemo<AssistantSessionContextValue>(
    () => ({
      messages,
      isStreaming,
      error,
      pendingConsent,
      selection,
      toolsEnabled,
      returnPath,
      setSelection,
      setToolsEnabled,
      sendPrompt,
      sendConsent: typedSendConsent,
      abort,
      reset,
    }),
    [
      abort,
      error,
      isStreaming,
      messages,
      pendingConsent,
      reset,
      returnPath,
      selection,
      sendPrompt,
      toolsEnabled,
      typedSendConsent,
    ],
  );

  return (
    <AssistantSessionContext.Provider value={value}>
      {children}
    </AssistantSessionContext.Provider>
  );
}
