import * as React from "react";
import type { TFunction } from "i18next";
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
  type AssistantFocus,
  type AssistantHandoff,
  type AssistantScreenContext,
  type AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import { pickChatAttachmentSource } from "@/lib/filePicker";
import { useFastModeSupport } from "@/components/ai/useFastModeSupport";
import { useProviderDefaultEffort } from "@/components/ai/useProviderDefaultEffort";
import {
  useActiveBookScope,
  type AssistantBookScope,
} from "@/components/ai/useActiveBookScope";
import { currentAssistantScreenContext } from "@/components/ai/assistantScreenContext";
import {
  type AiChatMessage,
  type AiChatRequest,
  type AiToolConsentDecision,
  type StoredChatEntry,
  type StoredChatMessageShape,
  storedChatEntries,
  useAiChatStream,
} from "@/daemon/stream";
import { getTransport, makeDaemonRequestId } from "@/daemon/transport";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { useUiStore } from "@/store/ui";

/**
 * An "Ask about this" pin and the daemon session (book) it was taken in. A pin
 * only applies while that session is still current.
 */
interface Pin {
  focus: AssistantFocus;
  daemonSession: number;
}

/**
 * The pin to send now: none when the pin was taken in a book that has since
 * been switched away. Read from the store at send time, so it holds even when
 * the switch lands in the same render as the send.
 */
function livePin(pin: Pin | null): AssistantFocus | null {
  return pin && pin.daemonSession === useUiStore.getState().daemonSession
    ? pin.focus
    : null;
}

/**
 * A question held in the queue (an answer is streaming, or this book's
 * provider settings have not seeded the effort yet). It keeps the page it was
 * asked on and the "Ask about this" pin that was active then, so navigating,
 * or pinning or clearing another item, before it runs cannot retarget it. A
 * handoff keeps the screen context of the page that made it, and no pin.
 * Either way it belongs to the book (daemon session) it was asked in and
 * never runs in another.
 */
interface QueuedPrompt {
  prompt: string;
  pin: Pin | null;
  /**
   * The page's typed context when it was submitted (route, capabilities, and
   * counts with the book state they were counted on, which the daemon
   * checks), or a handoff's own.
   */
  context: AssistantScreenContext;
  /** A page's own request: its text was written from that page's data. */
  handoff: boolean;
  daemonSession: number;
  /** The book's ids for the daemon's check; the open book's when not known yet. */
  scope: AssistantBookScope | null;
}

/**
 * One rule for a request made in another book (queued, held, or parked in
 * the composer): it is never sent here. The user's own text returns to the
 * composer; a page's handoff (Source of Funds, PSBT, datasets, chain
 * analysis, the quarantine fix) is dropped with its text, which carries that
 * book's transactions, recipe, grants or snapshot ids. A notice says which.
 */
function noticeBookSwitched(
  t: TFunction<"assistant">,
  { returned, cleared }: { returned: boolean; cleared: boolean },
) {
  useUiStore.getState().addNotification({
    title: t("queue.bookSwitched.title"),
    body: t(
      returned && cleared
        ? "queue.bookSwitched.bodyBoth"
        : cleared
          ? "queue.bookSwitched.bodyCleared"
          : "queue.bookSwitched.bodyReturned",
    ),
    tone: "warning",
    dedupeKey: "assistant-book-switched",
  });
}

/**
 * Put the text of queued requests that can no longer run back in the
 * composer, once each: a text already in the draft is not added again.
 */
function returnToDraft(prompts: string[]) {
  const drafts = useAssistantDraftStore.getState();
  const current = drafts.draft.trim();
  const missing = prompts.filter((prompt) => !current.includes(prompt));
  if (missing.length === 0) return;
  drafts.replaceDraft([current, ...missing].filter(Boolean).join("\n\n"));
}

interface StoredSessionShape {
  id?: string;
  messages?: StoredChatMessageShape[];
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
  // A provider's stored default effort seeds the session when it is selected.
  // The context hands out the hook's setter, so a level the user picks is
  // recorded as chosen and survives provider switches.
  const { setThinkingEffort: chooseThinkingEffort, effortSeeded } =
    useProviderDefaultEffort({
      selection,
      thinkingEffort,
      setThinkingEffort,
    });
  // Until this book's provider settings have seeded the effort, a send would
  // carry the previous book's effort (or auto instead of the default): hold it.
  // Every turn names the book it was asked in (`expected_scope`), so sends
  // also wait until the open book's ids are known.
  const bookScope = useActiveBookScope();
  const bookScopeRef = React.useRef(bookScope);
  bookScopeRef.current = bookScope;
  const ready = effortSeeded && bookScope !== null;
  const sendBlockedReason = !effortSeeded
    ? t("composer.loadingProviderSettings")
    : !bookScope
      ? t("composer.loadingBook")
      : null;
  // Session-scoped like the effort: fast mode is billed at a higher rate, so
  // it never carries over into a later app launch on its own.
  const [fastModeRequested, setFastMode] = React.useState(false);
  const fastModeSupport = useFastModeSupport(selection);
  const fastMode = fastModeRequested && fastModeSupport.supported;
  // Switching to a model without fast mode drops the request, as efforts do.
  React.useEffect(() => {
    if (fastModeRequested && fastModeSupport.resolved && !fastModeSupport.supported) {
      setFastMode(false);
    }
  }, [fastModeRequested, fastModeSupport.resolved, fastModeSupport.supported]);
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
  // Text placed in the composer from elsewhere replaces any parked handoff.
  const setAssistantDraft = useAssistantDraftStore((state) => state.replaceDraft);
  const [queuedPrompts, setQueuedPrompts] = React.useState<QueuedPrompt[]>([]);
  const [incognito, setIncognito] = React.useState(false);
  const [attachment, setAttachment] =
    React.useState<AssistantAttachment | null>(null);
  // "Ask about this" pin. It belongs to the book it was taken in, so a book
  // or daemon switch drops it.
  // Sends check the pin's session synchronously (`livePin`); this only
  // removes the chip once the switch has rendered.
  const [pin, setPin] = React.useState<Pin | null>(null);
  const focus = pin && pin.daemonSession === daemonSession ? pin.focus : null;
  // Read through a ref, so the effect below runs for a book switch only and
  // a language change (a new `t`) never clears the pin.
  const tRef = React.useRef(t);
  React.useEffect(() => {
    tRef.current = t;
  }, [t]);
  React.useEffect(() => {
    setPin(null);
    // A handoff parked in the composer by the previous book goes with it
    // (the send path checks again, in case a send races this effect).
    const drafts = useAssistantDraftStore.getState();
    if (drafts.handoff && drafts.handoff.daemonSession !== daemonSession) {
      drafts.replaceDraft("");
      noticeBookSwitched(tRef.current, { returned: false, cleared: true });
    }
  }, [daemonSession]);
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
      // What a submitted question was asked under: the pin then (dropped if
      // its book is gone) and the page it was asked on, or a handoff's own
      // page context. Omitted means the pin and the page now.
      asked?: Pick<QueuedPrompt, "pin" | "context" | "handoff" | "scope">,
    ) => {
      if (!selection?.model) return;
      // The book this turn was asked in; the daemon refuses it in another.
      const expectedScope = evidence
        ? { workspace_id: evidence.request.workspace_id, profile_id: evidence.request.profile_id }
        : asked?.scope ?? bookScopeRef.current ?? undefined;
      turnStampRef.current = { generation: generationRef.current,
        daemonSession: useUiStore.getState().daemonSession, promptRevision: promptRevisionRef.current };
      const baseConversation = baseMessages.filter((message) => message.role !== "system");
      const priorMessages: AiChatRequest["messages"] = baseConversation.map((message) => ({
        role: message.role,
        content: message.content,
      }));
      const next: AiChatRequest["messages"] = [
        ...priorMessages,
        { role: "user", content: prompt },
      ];
      const seedHistory = seedHistoryPendingRef.current && activeSession === null;
      seedHistoryPendingRef.current = false;
      // A fork's copied answers keep their notices in the new session. They
      // travel beside the messages, never in them, so the model never sees them.
      const seedNotices = seedHistory
        ? baseConversation.flatMap((message, index) =>
            message.role === "assistant" && message.notices?.length
              ? [{ index, notices: message.notices }]
              : [],
          )
        : [];
      void Promise.resolve(send(
        {
          provider: selection.provider,
          model: selection.model,
          messages: next,
          options:
            thinkingEffort === "auto" && !fastMode
              ? undefined
              : {
                  ...(thinkingEffort === "auto"
                    ? {}
                    : { reasoning_effort: thinkingEffort }),
                  ...(fastMode ? { fast_mode: true } : {}),
                },
          toolsEnabled: true,
          systemPromptKind: "kassiber",
          sessionId: activeSession,
          persist: incognito && activeSession === null ? false : "auto",
          seedHistory,
          ...(seedNotices.length ? { seedNotices } : {}),
          // Token only. `label` is the user's own words about the file (the
          // CLI's --file-context); sending the filename there would have the
          // daemon tell the model "the user describes it as: export.csv", and
          // the daemon already names the file from the staging grant.
          attachment: (evidence ? evidence.attachment : attachment)
            ? { token: (evidence ? evidence.attachment : attachment)!.token } : undefined,
          expectedScope,
          // An evidence continuation or a page's handoff brings its own
          // context and tool pack; a pin never replaces it. A pin does
          // replace the page a question was asked on.
          screenContext: (evidence ? evidenceContinuationScreenContext(evidence.request) : null)
            ?? (asked?.handoff ? asked.context : null)
            ?? livePin(asked ? asked.pin : pin)?.context
            ?? asked?.context
            ?? currentAssistantScreenContext(screenContext),
        },
        prompt,
      )).then((outcome) => {
        // Refused because another book was open when it arrived: the same
        // rule as a client-side mismatch. A handoff or evidence continuation
        // was written from that book's data and is dropped; the user's own
        // text returns to the composer.
        if (!outcome?.refused) return;
        const generated = Boolean(evidence || asked?.handoff);
        if (!generated) returnToDraft([prompt]);
        noticeBookSwitched(tRef.current, {
          returned: !generated,
          cleared: generated,
        });
      });
    },
    [attachment, fastMode, pin, incognito, screenContext, selection, send, thinkingEffort],
  );

  const latestRef = React.useRef({ messages, sessionId, isStreaming, hasModel: Boolean(selection?.model) && ready, queued: queuedPrompts.length > 0, runTurn });
  React.useEffect(() => { latestRef.current = { messages, sessionId, isStreaming, hasModel: Boolean(selection?.model) && ready, queued: queuedPrompts.length > 0, runTurn }; },
    [messages, sessionId, isStreaming, selection?.model, ready, queuedPrompts.length, runTurn]);

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
    (item: QueuedPrompt) => {
      runTurn(item.prompt, messages, sessionId, undefined, {
        pin: item.pin,
        context: item.context,
        handoff: item.handoff,
        scope: item.scope,
      });
    },
    [messages, runTurn, sessionId],
  );

  const sendPrompt = React.useCallback(
    (prompt: string, explicitHandoff?: AssistantHandoff) => {
      const trimmed = prompt.trim();
      if (!trimmed || !selection?.model) return;
      let handoff = explicitHandoff;
      // A handoff parked in the composer (no model was chosen when it was
      // made) is sent as that handoff. One parked in another book is not
      // sent at all, and its text, which carries that book's data, goes too.
      if (!handoff) {
        const parked = useAssistantDraftStore.getState().takeHandoff();
        if (parked && parked.daemonSession !== useUiStore.getState().daemonSession) {
          useAssistantDraftStore.getState().replaceDraft("");
          noticeBookSwitched(t, { returned: false, cleared: true });
          return;
        }
        if (parked) handoff = { context: parked.context };
      }
      promptRevisionRef.current += 1;
      // Captured now, so a question that waits in the queue still runs with
      // the page it was asked on. A page's handoff runs with the page it came
      // from and replaces the pin rather than being narrowed by it.
      const item: QueuedPrompt = {
        prompt: trimmed,
        pin: handoff ? null : pin,
        context: handoff?.context ?? currentAssistantScreenContext(screenContext),
        handoff: Boolean(handoff),
        daemonSession: useUiStore.getState().daemonSession,
        scope: bookScopeRef.current,
      };
      if (handoff) setPin(null);
      // Also held while the effort is not yet seeded for this book; the queue
      // sends it once it is.
      if (isStreaming || !ready) {
        setQueuedPrompts((current) => [...current, item]);
        return;
      }
      dispatchPrompt(item);
    },
    [dispatchPrompt, ready, pin, isStreaming, screenContext, selection, t],
  );

  React.useEffect(() => {
    if (isStreaming || queuedPrompts.length === 0 || !ready) return;
    if (!selection?.model) return;
    // Checked here, as it is sent, against the book open now: a request
    // queued in another book never runs in this one (noticeBookSwitched).
    const daemonSessionNow = useUiStore.getState().daemonSession;
    const stale = queuedPrompts.filter(
      (item) => item.daemonSession !== daemonSessionNow,
    );
    if (stale.length > 0) {
      setQueuedPrompts((current) =>
        current.filter((item) => item.daemonSession === daemonSessionNow),
      );
      // A handoff's text was written from its page's data, so it is dropped
      // rather than returned.
      const own = stale.filter((item) => !item.handoff);
      returnToDraft(own.map((item) => item.prompt));
      noticeBookSwitched(t, {
        returned: own.length > 0,
        cleared: own.length < stale.length,
      });
      return;
    }
    const [next] = queuedPrompts;
    setQueuedPrompts((current) => current.slice(1));
    dispatchPrompt(next);
  }, [dispatchPrompt, ready, isStreaming, queuedPrompts, selection, t]);

  const queuedPromptTexts = React.useMemo(
    () => queuedPrompts.map((item) => item.prompt),
    [queuedPrompts],
  );

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
    setPin(null);
    reset();
  }, [clearEvidence, reset]);

  const askAbout = React.useCallback(
    (nextFocus: AssistantFocus, prompt: string) => {
      setPin({
        focus: nextFocus,
        daemonSession: useUiStore.getState().daemonSession,
      });
      // A draft, not a send: the user reviews the question (and which model it
      // goes to) before anything leaves the device.
      setAssistantDraft(prompt);
      useUiStore.getState().openAssistantPanel({ focusComposer: true });
    },
    [setAssistantDraft],
  );
  const clearFocus = React.useCallback(() => setPin(null), []);

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
      const entries = storedChatEntries(envelope.data?.messages ?? []);
      setQueuedPrompts([]);
      setIncognito(false);
      // The grant belongs to the conversation that asked for it. Loading a
      // different saved chat must not carry the previous chat's file into it —
      // the next turn would analyze a file this conversation never mentioned.
      setAttachment(null);
      setPin(null);
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
        .map((message) => ({
          role: message.role,
          content: message.content,
          ...(message.notices?.length ? { notices: message.notices } : {}),
        }));
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
        ...(message.notices?.length ? { notices: message.notices } : {}),
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
      if (!ready) {
        // This book's provider settings (or its ids) are not loaded yet: hold
        // the edited prompt in the queue, which sends it from the rewound
        // conversation once they are, like any other prompt, with the pin
        // and the page that are active now.
        setQueuedPrompts([
          {
            prompt: trimmed,
            pin,
            context: currentAssistantScreenContext(screenContext),
            handoff: false,
            daemonSession: useUiStore.getState().daemonSession,
            scope: bookScopeRef.current,
          },
        ]);
        return;
      }
      runTurn(trimmed, priorMessages, null);
    },
    [
      clearEvidence,
      ready,
      isStreaming,
      messages,
      loadConversation,
      pin,
      runTurn,
      screenContext,
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
      queuedPrompts: queuedPromptTexts,
      selection,
      thinkingEffort,
      fastMode,
      returnPath: screenContext.route,
      sessionId,
      incognito,
      attachment,
      setSelection,
      setThinkingEffort: chooseThinkingEffort,
      sendBlockedReason,
      setFastMode,
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
      focus,
      askAbout,
      clearFocus,
    }),
    [
      askAbout,
      clearFocus,
      focus,
      evidenceRequests, openEvidenceRequest, continueEvidenceRequest,
      abort,
      attachFile,
      attachment,
      branchFromMessage,
      chooseThinkingEffort,
      clearAttachment,
      editUserMessage,
      clearChat,
      error,
      fastMode,
      forgetSession,
      incognito,
      isStreaming,
      messages,
      pendingConsent,
      queuedPromptTexts,
      resumeSession,
      screenContext.route,
      sessionId,
      setSelection,
      selection,
      sendBlockedReason,
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
