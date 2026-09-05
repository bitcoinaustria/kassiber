import * as React from "react";
import { useTranslation } from "react-i18next";
import { evidenceRequest, evidenceRevalidationRequest, evidenceAttachmentOptions, evidenceContinuationScreenContext, sameHandoffContext, canAutoContinueEvidence,
  canStartEvidenceHandoff, canResumeEvidenceHandoff, isActiveEvidenceHandoff,
  type EvidenceRequest, type EvidenceRequestState, type HandoffStamp } from "./evidenceRequest";
import type { ConnectionSetupOutcome } from "@/components/kb/connectionSetupOutcome";
const AddConnectionDialog = React.lazy(() => import("@/components/kb/AddConnectionDialog").then((module) => ({ default: module.AddConnectionDialog })));
type EvidenceOutcome = "received" | "partial" | "attached" | "unavailable";
interface PendingEvidence { request: EvidenceRequest; origin: HandoffStamp; outcome?: EvidenceOutcome; attachment?: AssistantAttachment }


import {
  AssistantSessionContext,
  type AssistantAttachment,
  type AssistantScreenContext,
  type AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import { pickChatAttachmentSource } from "@/lib/filePicker";
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
  const { t } = useTranslation("assistant");
  const daemonSession = useUiStore((state) => state.daemonSession);
  const generationRef = React.useRef(0);
  const promptRevisionRef = React.useRef(0);
  const turnStampRef = React.useRef<HandoffStamp>({ generation: 0, daemonSession, promptRevision: 0 });
  const evidenceOriginsRef = React.useRef(new Map<string, HandoffStamp>());
  const continuedEvidenceRef = React.useRef(new Set<string>());
  const pendingEvidenceRef = React.useRef<PendingEvidence | null>(null);
  const [evidenceDialog, setEvidenceDialog] = React.useState<PendingEvidence | null>(null);
  const [evidenceRequests, setEvidenceRequests] = React.useState<Record<string, EvidenceRequestState>>({});
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
  const setAssistantDraft = useAssistantDraftStore((state) => state.setDraft);
  const [queuedPrompts, setQueuedPrompts] = React.useState<string[]>([]);
  const [incognito, setIncognito] = React.useState(false);
  const [attachment, setAttachment] =
    React.useState<AssistantAttachment | null>(null);
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
      evidence?: { request: EvidenceRequest; attachment?: AssistantAttachment },
    ) => {
      if (!selection?.model) return;
      turnStampRef.current = { generation: generationRef.current,
        daemonSession: useUiStore.getState().daemonSession, promptRevision: promptRevisionRef.current };
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
          systemPromptKind: "kassiber",
          sessionId: activeSession,
          persist: incognito && activeSession === null ? false : "auto",
          seedHistory,
          // Token only. `label` is the user's own words about the file (the
          // CLI's --file-context); sending the filename there would have the
          // daemon tell the model "the user describes it as: export.csv", and
          // the daemon already names the file from the staging grant.
          attachment: (evidence ? evidence.attachment : attachment)
            ? { token: (evidence ? evidence.attachment : attachment)!.token } : undefined,
          expectedScope: evidence ? { workspace_id: evidence.request.workspace_id, profile_id: evidence.request.profile_id } : undefined,
          screenContext: (evidence ? evidenceContinuationScreenContext(evidence.request) : null)
            ?? currentAssistantScreenContext(screenContext),
        },
        prompt,
      );
    },
    [attachment, incognito, screenContext, selection, send, thinkingEffort],
  );

  const latestRef = React.useRef({ messages, sessionId, isStreaming, hasModel: Boolean(selection?.model), queued: queuedPrompts.length > 0, runTurn });
  React.useEffect(() => { latestRef.current = { messages, sessionId, isStreaming, hasModel: Boolean(selection?.model), queued: queuedPrompts.length > 0, runTurn }; },
    [messages, sessionId, isStreaming, selection?.model, queuedPrompts.length, runTurn]);

  const stamp = React.useCallback((): HandoffStamp => ({
    generation: generationRef.current,
    daemonSession: useUiStore.getState().daemonSession,
    promptRevision: promptRevisionRef.current,
  }), []);

  const clearEvidence = React.useCallback(() => {
    generationRef.current += 1;
    pendingEvidenceRef.current = null;
    evidenceOriginsRef.current.clear();
    continuedEvidenceRef.current.clear();
    setEvidenceDialog(null);
    setEvidenceRequests({});
  }, []);

  React.useEffect(() => {
    for (const message of messages) for (const call of message.toolCalls ?? []) {
      const request = evidenceRequest(call);
      if (request && !evidenceOriginsRef.current.has(request.request_id)) {
        evidenceOriginsRef.current.set(request.request_id, { ...turnStampRef.current });
      }
    }
  }, [messages]);

  React.useEffect(() => {
    setEvidenceRequests((states) => {
      const next = { ...states };
      for (const [id, origin] of evidenceOriginsRef.current) {
        if (!sameHandoffContext(origin, stamp())) next[id] = { status: "stale" };
      }
      return next;
    });
    if (pendingEvidenceRef.current && !sameHandoffContext(pendingEvidenceRef.current.origin, stamp())) {
      pendingEvidenceRef.current = null;
      setEvidenceDialog(null);
    }
  }, [daemonSession, stamp]);

  const currentEvidence = React.useCallback((request: EvidenceRequest, origin: HandoffStamp) =>
    sameHandoffContext(origin, stamp()) && latestRef.current.messages.some((message) =>
      message.toolCalls?.some((call) => evidenceRequest(call)?.request_id === request.request_id)), [stamp]);

  const continueEvidenceRequest = React.useCallback((request: EvidenceRequest, unavailable = false) => {
    if (!canResumeEvidenceHandoff(request.request_id, pendingEvidenceRef.current, continuedEvidenceRef.current)) return;
    const origin = evidenceOriginsRef.current.get(request.request_id);
    if (!origin || !currentEvidence(request, origin)) {
      setEvidenceRequests((old) => ({ ...old, [request.request_id]: { status: "stale" } }));
      return;
    }
    if (unavailable) pendingEvidenceRef.current = { request, origin, outcome: "unavailable" };
    const latest = latestRef.current;
    if (!latest.hasModel || latest.isStreaming || latest.queued || useAssistantDraftStore.getState().draft.trim()) {
      setEvidenceRequests((old) => ({ ...old, [request.request_id]: { status: unavailable ? "unavailable" : "received" } }));
      return;
    }
    const pending = pendingEvidenceRef.current;
    const outcome = unavailable ? "unavailable" : pending?.outcome ?? "received";
    const nextAttachment = pending?.attachment;
    pendingEvidenceRef.current = null;
    continuedEvidenceRef.current.add(request.request_id);
    setEvidenceRequests((old) => ({ ...old, [request.request_id]: { status: "continuing" } }));
    const prompt = t(request.domain === "source_funds" ? "evidence.sourceFundsContinuePrompt" : "evidence.continuePrompt", {
      outcome: t(`evidence.outcome.${outcome}`),
      cases: request.cases.map((item) => item.case_id).join(", "),
    });
    latest.runTurn(prompt, latest.messages, latest.sessionId, { request, attachment: nextAttachment });
  }, [currentEvidence, t]);

  const completeEvidence = React.useCallback((pending: PendingEvidence, outcome: EvidenceOutcome,
    nextAttachment?: AssistantAttachment) => {
    if (!isActiveEvidenceHandoff(pendingEvidenceRef.current, pending, stamp()) || !currentEvidence(pending.request, pending.origin)) return;
    pending.outcome = outcome;
    pending.attachment = nextAttachment;
    if (nextAttachment) setAttachment(nextAttachment);
    setEvidenceDialog(null);
    setEvidenceRequests((old) => ({ ...old, [pending.request.request_id]: { status: outcome === "partial" ? "partial" : "received" } }));
    const latest = latestRef.current;
    if (canAutoContinueEvidence(pending.origin, stamp(), latest.isStreaming || !latest.hasModel,
      Boolean(useAssistantDraftStore.getState().draft.trim()), latest.queued)) {
      continueEvidenceRequest(pending.request);
    }
  }, [continueEvidenceRequest, currentEvidence, stamp]);

  const openEvidenceRequest = React.useCallback(async (request: EvidenceRequest) => {
    if (latestRef.current.isStreaming) return;
    if (!canStartEvidenceHandoff(request.request_id, pendingEvidenceRef.current, continuedEvidenceRef.current)) return;
    const origin = evidenceOriginsRef.current.get(request.request_id);
    if (!origin || !currentEvidence(request, origin)) {
      setEvidenceRequests((old) => ({ ...old, [request.request_id]: { status: "stale" } }));
      return;
    }
    const pending: PendingEvidence = { request, origin: { ...origin, promptRevision: promptRevisionRef.current } };
    pendingEvidenceRef.current = pending;
    setEvidenceRequests((old) => ({ ...old, [request.request_id]: { status: "opening" } }));
    try {
      // Revalidate current cases before any local picker or setup egress.
      const checked = await getTransport().invoke(evidenceRevalidationRequest(request));
      if (checked.kind === "error" || checked.error) throw new Error(checked.error?.message ?? t("evidence.stale"));
      const packet = checked.data as { request_id?: unknown } | undefined;
      if (packet?.request_id !== request.request_id) throw new Error(t("evidence.stale"));
      if (!isActiveEvidenceHandoff(pendingEvidenceRef.current, pending, stamp()) || !currentEvidence(request, origin)) return;
      if (request.action === "attach_evidence") {
        const selected = await pickChatAttachmentSource(evidenceAttachmentOptions(request));
        if (!isActiveEvidenceHandoff(pendingEvidenceRef.current, pending, stamp()) || !currentEvidence(request, origin)) return;
        if (!selected) {
          pendingEvidenceRef.current = null;
          setEvidenceRequests((old) => ({ ...old, [request.request_id]: { status: "idle" } }));
          return;
        }
        if (typeof selected.attachment_id !== "string" || !selected.attachment_id
          || selected.transaction_id !== request.cases[0].transaction_id) {
          throw new Error(t("evidence.attachmentUnverified"));
        }
        completeEvidence(pending, "attached", { token: selected.document_token,
          filename: selected.source.filename, kind: selected.source.kind, sizeBytes: selected.source.size_bytes });
      } else setEvidenceDialog(pending);
    } catch (error) {
      if (!isActiveEvidenceHandoff(pendingEvidenceRef.current, pending, stamp()) || !currentEvidence(request, origin)) return;
      pendingEvidenceRef.current = null;
      setEvidenceRequests((old) => ({ ...old, [request.request_id]: {
        status: "error", error: error instanceof Error ? error.message : t("evidence.failed"),
      } }));
    }
  }, [completeEvidence, currentEvidence, stamp, t]);

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
      promptRevisionRef.current += 1;
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
    clearEvidence();
    setQueuedPrompts([]);
    seedHistoryPendingRef.current = false;
    // The grant belongs to the conversation that asked for it; a new chat must
    // not silently keep analyzing the previous chat's file.
    setAttachment(null);
    reset();
  }, [clearEvidence, reset]);

  const attachFile = React.useCallback(async () => {
    const origin = stamp();
    const selected = await pickChatAttachmentSource();
    if (!sameHandoffContext(origin, stamp())) return;
    if (!selected) return; // cancelled, or no picker in this runtime
    setAttachment({
      token: selected.document_token,
      filename: selected.source.filename,
      kind: selected.source.kind,
      sizeBytes: selected.source.size_bytes,
    });
  }, [stamp]);

  const clearAttachment = React.useCallback(() => setAttachment(null), []);

  const resumeSession = React.useCallback(
    async (targetSessionId: string) => {
      if (isStreaming) return;
      clearEvidence();
      const resumeGeneration = generationRef.current;
      const envelope = await getTransport().invoke<StoredSessionShape>({
        kind: "ui.chat.sessions.get",
        request_id: makeDaemonRequestId(),
        args: { session_id: targetSessionId },
      });
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(
          envelope.error?.message ?? "Could not load the chat session",
        );
      }
      if (generationRef.current !== resumeGeneration) return;
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
      // The grant belongs to the conversation that asked for it. Loading a
      // different saved chat must not carry the previous chat's file into it —
      // the next turn would analyze a file this conversation never mentioned.
      setAttachment(null);
      // Drop any half-typed draft before binding the resumed (persisted)
      // session — otherwise text typed while Incognito would ride into the
      // loaded chat and be stored on the next submit.
      setAssistantDraft("");
      seedHistoryPendingRef.current = false;
      loadConversation(entries, envelope.data?.id ?? targetSessionId);
    },
    [clearEvidence, isStreaming, loadConversation, setAssistantDraft],
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
      clearEvidence();
      // Preserve the current Incognito choice — forking must never silently
      // flip a private conversation into one that persists.
      setQueuedPrompts([]);
      // Explicit fork: the next send may persist this seeded prefix.
      seedHistoryPendingRef.current = true;
      loadConversation(entries, null);
    },
    [clearEvidence, isStreaming, messages, loadConversation],
  );

  const editUserMessage = React.useCallback(
    (messageId: string, nextContent?: string) => {
      if (isStreaming) return;
      const index = messages.findIndex((message) => message.id === messageId);
      if (index < 0) return;
      const target = messages[index];
      if (target.role !== "user") return;
      clearEvidence();
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
      clearEvidence,
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
      evidenceRequests, openEvidenceRequest, continueEvidenceRequest,
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
      attachment,
      setSelection,
      setThinkingEffort,
      setIncognito,
      attachFile,
      clearAttachment,
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
      evidenceRequests, openEvidenceRequest, continueEvidenceRequest,
      abort,
      attachFile,
      attachment,
      branchFromMessage,
      clearAttachment,
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
      {evidenceDialog ? <React.Suspense fallback={null}>
        <AddConnectionDialog key={evidenceDialog.request.request_id} open
          initialSourceId={evidenceDialog.request.action === "import_history" ? null : "descriptor"}
          initialTargetWalletId={evidenceDialog.request.action === "import_history"
            ? (new Set(evidenceDialog.request.cases.map((item) => item.wallet_id)).size === 1
              ? evidenceDialog.request.cases[0].wallet_id ?? null : null) : undefined}
          scopeBoundary={{ expectedScope: { workspace_id: evidenceDialog.request.workspace_id,
            profile_id: evidenceDialog.request.profile_id }, daemonSession: evidenceDialog.origin.daemonSession,
            isCurrent: () => pendingEvidenceRef.current === evidenceDialog && currentEvidence(evidenceDialog.request, evidenceDialog.origin) }}
          onCompleted={(outcome: ConnectionSetupOutcome) => completeEvidence(evidenceDialog, outcome.status === "partial" ? "partial" : "received")}
          onOpenChange={(open) => {
            if (open || pendingEvidenceRef.current !== evidenceDialog || evidenceDialog.outcome) return;
            pendingEvidenceRef.current = null;
            setEvidenceDialog(null);
            setEvidenceRequests((old) => ({ ...old, [evidenceDialog.request.request_id]: { status: "idle" } }));
          }} />
      </React.Suspense> : null}
    </AssistantSessionContext.Provider>
  );
}
