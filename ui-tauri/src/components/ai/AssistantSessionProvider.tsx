import * as React from "react";

import {
  AssistantSessionContext,
  type AssistantScreenContext,
  type AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import { currentAssistantScreenContext } from "@/components/ai/assistantScreenContext";
import {
  type AiChatMessage,
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
  screenContext: AssistantScreenContext;
}

export function AssistantSessionProvider({
  children,
  screenContext,
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
  // Set by branch/edit; consumed on the next send so the daemon persists the
  // seeded prefix for that fork only. A bare detached conversation (history
  // toggled, session deleted) never carries it, so its prior turns are not
  // backfilled into a new session.
  const seedHistoryPendingRef = React.useRef(false);

  // Runs one chat turn against an explicit conversation base + session, so
  // callers that rewind history (edit) can regenerate atomically without
  // waiting for `messages`/`sessionId` state to settle first.
  const runTurn = React.useCallback(
    (
      prompt: string,
      baseMessages: AiChatMessage[],
      activeSession: string | null,
    ) => {
      if (!selection?.model) return;
      const priorMessages: AiChatRequest["messages"] = baseMessages
        .filter((message) => message.role !== "system")
        .map((message) => ({
          role: message.role,
          content: message.content,
        }));
      const next: AiChatRequest["messages"] = [
        ...priorMessages,
        { role: "user", content: prompt },
      ];
      const seedHistory = seedHistoryPendingRef.current && activeSession === null;
      seedHistoryPendingRef.current = false;
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
          sessionId: activeSession,
          persist: incognito && activeSession === null ? false : "auto",
          seedHistory,
          screenContext: currentAssistantScreenContext(screenContext),
        },
        prompt,
      );
    },
    [incognito, screenContext, selection, send, thinkingEffort],
  );

  const dispatchPrompt = React.useCallback(
    (prompt: string) => {
      runTurn(prompt, messages, sessionId);
    },
    [messages, runTurn, sessionId],
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
    seedHistoryPendingRef.current = false;
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
      seedHistoryPendingRef.current = false;
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
      // Explicit fork: the next send may persist this seeded prefix.
      seedHistoryPendingRef.current = true;
      loadConversation(entries, null);
    },
    [isStreaming, messages, loadConversation],
  );

  const editUserMessage = React.useCallback(
    (messageId: string, nextContent?: string) => {
      if (isStreaming) return;
      const index = messages.findIndex((message) => message.id === messageId);
      if (index < 0) return;
      const target = messages[index];
      if (target.role !== "user") return;
      // Everything strictly before the edited prompt is the conversation we
      // keep; the edited turn and all downstream messages are regenerated.
      const priorMessages = messages
        .slice(0, index)
        .filter(
          (message): message is (typeof messages)[number] & {
            role: "user" | "assistant";
          } =>
            (message.role === "user" || message.role === "assistant") &&
            typeof message.content === "string" &&
            message.content.length > 0,
        );
      const entries: StoredChatEntry[] = priorMessages.map((message) => ({
        role: message.role,
        content: message.content,
      }));
      if (nextContent === undefined) {
        // Legacy rollback path (no inline edit): rewind and drop the prompt
        // back into the composer for a manual resend. Kept for callers that
        // don't drive the inline editor.
        setQueuedPrompts([]);
        // Explicit fork: the manual resend may persist this seeded prefix.
        seedHistoryPendingRef.current = true;
        loadConversation(entries, null);
        setAssistantDraft(target.content);
        return;
      }
      const trimmed = nextContent.trim();
      if (!trimmed || !selection?.model) return;
      // Inline edit confirm: rewind to just before the edited prompt, then
      // regenerate from the edited text in one atomic turn. Resending starts a
      // fresh, unsaved turn (null session) so the original conversation stays
      // intact in History; the current Incognito choice is preserved.
      setQueuedPrompts([]);
      // Explicit fork: the next send may persist this seeded prefix.
      seedHistoryPendingRef.current = true;
      loadConversation(entries, null);
      runTurn(trimmed, priorMessages, null);
    },
    [
      isStreaming,
      messages,
      loadConversation,
      runTurn,
      selection,
      setAssistantDraft,
    ],
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
      returnPath: screenContext.route,
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
      screenContext.route,
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
