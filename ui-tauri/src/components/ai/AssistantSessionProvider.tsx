import * as React from "react";

import {
  AssistantSessionContext,
  type AssistantReturnPath,
  type AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import {
  type AiChatRequest,
  type AiToolConsentDecision,
  type StoredChatEntry,
  useAiChatStream,
} from "@/daemon/stream";
import { getTransport, makeDaemonRequestId } from "@/daemon/transport";
import { useUiStore } from "@/store/ui";

interface StoredSessionShape {
  id?: string;
  messages?: { role?: string; content?: string }[];
}

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
    sessionId,
    loadConversation,
    forgetSession,
  } = useAiChatStream();
  const dataMode = useUiStore((state) => state.dataMode);
  const [queuedPrompts, setQueuedPrompts] = React.useState<string[]>([]);
  const [incognito, setIncognito] = React.useState(false);

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
          sessionId,
          persist: incognito ? false : "auto",
        },
        prompt,
      );
    },
    [incognito, messages, selection, send, sessionId, thinkingEffort],
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

  const resumeSession = React.useCallback(
    async (targetSessionId: string) => {
      if (isStreaming) return;
      const envelope = await getTransport(dataMode).invoke<StoredSessionShape>({
        kind: "ui.chat.sessions.get",
        request_id: makeDaemonRequestId(),
        args: { session_id: targetSessionId },
      });
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(
          envelope.error?.message ?? "Could not load the chat session",
        );
      }
      const entries: StoredChatEntry[] = (envelope.data?.messages ?? [])
        .filter(
          (message): message is { role: "user" | "assistant"; content: string } =>
            (message.role === "user" || message.role === "assistant") &&
            typeof message.content === "string" &&
            message.content.length > 0,
        )
        .map((message) => ({ role: message.role, content: message.content }));
      setQueuedPrompts([]);
      loadConversation(entries, envelope.data?.id ?? targetSessionId);
    },
    [dataMode, isStreaming, loadConversation],
  );

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
      sessionId,
      incognito,
      setSelection,
      setThinkingEffort,
      setIncognito,
      sendPrompt,
      sendConsent: typedSendConsent,
      abort,
      reset: clearChat,
      resumeSession,
      forgetSession,
    }),
    [
      abort,
      clearChat,
      error,
      forgetSession,
      incognito,
      isStreaming,
      messages,
      pendingConsent,
      queuedPrompts,
      resumeSession,
      returnPath,
      sessionId,
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
