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
import { useAssistantDraftStore } from "@/store/assistantDraft";
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
  const setAssistantDraft = useAssistantDraftStore((state) => state.setDraft);
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
          persist: incognito && sessionId === null ? false : "auto",
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
      setIncognito(false);
      // Drop any half-typed draft before binding the resumed (persisted)
      // session — otherwise text typed while Incognito would ride into the
      // loaded chat and be stored on the next submit.
      setAssistantDraft("");
      loadConversation(entries, envelope.data?.id ?? targetSessionId);
    },
    [dataMode, isStreaming, loadConversation, setAssistantDraft],
  );

  const branchFromMessage = React.useCallback(
    (messageId: string) => {
      if (isStreaming) return;
      const index = messages.findIndex((message) => message.id === messageId);
      if (index < 0) return;
      // Seed a fresh, unsaved conversation with history up to and including the
      // selected message. A null sessionId means the next turn spins up a new
      // persisted session, so the original chat stays intact in History.
      const entries: StoredChatEntry[] = messages
        .slice(0, index + 1)
        .filter(
          (message): message is (typeof messages)[number] & {
            role: "user" | "assistant";
          } =>
            (message.role === "user" || message.role === "assistant") &&
            typeof message.content === "string" &&
            message.content.length > 0,
        )
        .map((message) => ({ role: message.role, content: message.content }));
      if (entries.length === 0) return;
      // Preserve the current Incognito choice — forking must never silently
      // flip a private conversation into one that persists.
      setQueuedPrompts([]);
      loadConversation(entries, null);
    },
    [isStreaming, messages, loadConversation],
  );

  const editUserMessage = React.useCallback(
    (messageId: string) => {
      if (isStreaming) return;
      const index = messages.findIndex((message) => message.id === messageId);
      if (index < 0) return;
      const target = messages[index];
      if (target.role !== "user") return;
      // Rewind to just before the edited prompt and drop its text back into the
      // composer. Resending starts a fresh, unsaved turn (null session), so the
      // original conversation stays intact in History.
      const entries: StoredChatEntry[] = messages
        .slice(0, index)
        .filter(
          (message): message is (typeof messages)[number] & {
            role: "user" | "assistant";
          } =>
            (message.role === "user" || message.role === "assistant") &&
            typeof message.content === "string" &&
            message.content.length > 0,
        )
        .map((message) => ({ role: message.role, content: message.content }));
      // Preserve the current Incognito choice (see branchFromMessage).
      setQueuedPrompts([]);
      loadConversation(entries, null);
      setAssistantDraft(target.content);
    },
    [isStreaming, messages, loadConversation, setAssistantDraft],
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
      branchFromMessage,
      editUserMessage,
      forgetSession,
    }),
    [
      abort,
      branchFromMessage,
      editUserMessage,
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
