import * as React from "react";
import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";
import { evidenceRequest, evidenceRevalidationRequest, evidenceAttachmentOptions, evidenceContinuationScreenContext, sameHandoffContext, canAutoContinueEvidence,
  canStartEvidenceHandoff, canResumeEvidenceHandoff, isActiveEvidenceHandoff,
  type EvidenceRequest, type EvidenceRequestState, type HandoffStamp } from "./evidenceRequest";
import type { ConnectionSetupOutcome } from "@/components/kb/connectionSetupOutcome";
const AddConnectionDialog = React.lazy(() => import("@/components/kb/AddConnectionDialog").then((module) => ({ default: module.AddConnectionDialog })));
type EvidenceOutcome = "received" | "partial" | "attached" | "unavailable";
interface PendingEvidence { request: EvidenceRequest; origin: HandoffStamp; outcome?: EvidenceOutcome; attachment?: StagedAttachment }


import {
  AssistantSessionContext,
  type AssistantAttachment,
  type AssistantChatFile,
  type AssistantFocus,
  type AssistantHandoff,
  type AssistantScreenContext,
  type AssistantSessionContextValue,
} from "@/components/ai/assistantSession";
import { pickChatAttachmentSource } from "@/lib/filePicker";
import {
  MAX_CHAT_ATTACHMENTS,
  MAX_CHAT_UPLOAD_BYTES,
  chatAttachmentKind,
  chatImageThumbnail,
  chatThumbnail,
  isChatAttachmentName,
} from "@/components/ai/chatAttachments";
import {
  ChatAttachmentError,
  stageDroppedChatAttachments,
  uploadChatAttachment,
  type DroppedChatAttachments,
} from "@/components/ai/chatAttachmentUpload";
import { useFastModeSupport } from "@/components/ai/useFastModeSupport";
import { useProviderDefaultEffort } from "@/components/ai/useProviderDefaultEffort";
import { sameBook, type BookStamp, type OpenBook } from "@/components/ai/bookStamp";
import { useActiveBookScope } from "@/components/ai/useActiveBookScope";
import {
  currentAssistantScreenContext,
  currentPageScreenContext,
} from "@/components/ai/assistantScreenContext";
import {
  type AiChatMessage,
  type AiChatRequest,
  type AiPermissionMode,
  type AiToolConsentDecision,
  type AiToolProfile,
  type StoredChatEntry,
  type StoredChatMessageShape,
  storedChatEntries,
  useAiChatStream,
} from "@/daemon/stream";
import { getTransport, makeDaemonRequestId } from "@/daemon/transport";
import { useAssistantDraftStore } from "@/store/assistantDraft";
import { DEFAULT_ASSISTANT_TOOL_PROFILE, useUiStore } from "@/store/ui";

/** A file attached to the conversation, with the book it was chosen in. */
type StagedAttachment = AssistantAttachment & { book: BookStamp };

/**
 * An "Ask about this" pin and the book it was taken in. A pin only applies
 * while that book is still the open one.
 */
interface Pin {
  focus: AssistantFocus;
  book: BookStamp;
}

/**
 * A question held in the queue (an answer is streaming, or this book's
 * provider settings have not seeded the effort yet). It keeps the page it was
 * asked on and the "Ask about this" pin that was active then, so navigating,
 * or pinning or clearing another item, before it runs cannot retarget it. A
 * handoff keeps the screen context of the page that made it, and no pin.
 * Either way it belongs to the book it was asked in, names that book to the
 * daemon, and never runs in another.
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
  /** The book it was asked in (a parked handoff's: the book it was parked in). */
  book: BookStamp;
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

/** The entries of a draft: the blocks that returned prompts are joined as. */
function draftEntries(text: string): string[] {
  return text
    .split(/\n[ \t]*\n/)
    .map((entry) => entry.trim())
    .filter(Boolean);
}

/** Whether `prompt` is in the draft as whole entries, not inside the user's text. */
function draftHoldsPrompt(draft: string, prompt: string): boolean {
  const entries = draftEntries(draft);
  const wanted = draftEntries(prompt);
  if (wanted.length === 0) return true;
  return entries.some((_, start) =>
    wanted.every((entry, offset) => entries[start + offset] === entry),
  );
}

/**
 * Put the text of queued requests that can no longer run back in the
 * composer, once each: a prompt already there as a whole entry is not added
 * again, but one that merely appears inside the user's own text is.
 */
function returnToDraft(prompts: string[]) {
  const drafts = useAssistantDraftStore.getState();
  const current = drafts.draft.trim();
  const missing = prompts.filter((prompt) => !draftHoldsPrompt(current, prompt));
  if (missing.length === 0) return;
  drafts.replaceDraft([current, ...missing].filter(Boolean).join("\n\n"));
}

/**
 * A turn that cannot run in the book open now: the user's own text returns
 * to the composer; a handoff or evidence continuation, written from the other
 * book's data, is dropped. A notice says which.
 */
function refuseInOtherBook(t: TFunction<"assistant">, prompt: string, generated: boolean) {
  if (!generated) returnToDraft([prompt]);
  noticeBookSwitched(t, { returned: !generated, cleared: generated });
}

/**
 * An action taken before the open book's ids are known: it cannot be stamped,
 * so it is not taken. The user's own text returns to the composer.
 */
function refuseBookLoading(t: TFunction<"assistant">, prompt?: string) {
  if (prompt) returnToDraft([prompt]);
  useUiStore.getState().addNotification({
    title: t("queue.bookLoading.title"),
    body: t(prompt ? "queue.bookLoading.bodyReturned" : "queue.bookLoading.body"),
    tone: "warning",
    dedupeKey: "assistant-book-loading",
  });
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
  // The book on screen at the last render.
  const openBookRef = React.useRef<OpenBook>({ daemonSession, scope: bookScope });
  openBookRef.current = { daemonSession, scope: bookScope };
  // The book open now, read when something is made or sent: a session switch
  // that has not rendered yet leaves its ids unknown.
  const bookNow = React.useCallback((): OpenBook => {
    const current = useUiStore.getState().daemonSession;
    const open = openBookRef.current;
    return { daemonSession: current, scope: open.daemonSession === current ? open.scope : null };
  }, []);
  // The stamp for content the user's action creates now (see BookStamp);
  // null while the open book's ids are not known.
  const captureBook = React.useCallback((): BookStamp | null => {
    const now = bookNow();
    return now.scope ? { daemonSession: now.daemonSession, scope: now.scope } : null;
  }, [bookNow]);
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
    pendingConsents,
    sendConsent,
    changePermissionMode,
    reset,
    sessionId,
    loadConversation,
    forgetSession: forgetStreamSession,
  } = useAiChatStream();
  // The chat's permission mode: a new chat starts in the Settings default, a
  // reopened chat in the mode it last used. Each turn names it to the daemon.
  const [permissionMode, setPermissionModeState] = React.useState<AiPermissionMode>(
    () => useUiStore.getState().assistantDefaultPermissionMode,
  );
  const permissionModeRef = React.useRef(permissionMode);
  permissionModeRef.current = permissionMode;
  // A saved chat remembers its mode once it has a session id (after its
  // first turn is stored), and again whenever the mode changes.
  React.useEffect(() => {
    if (sessionId) {
      useUiStore.getState().rememberAssistantChatPermissionMode(sessionId, permissionMode);
    }
  }, [permissionMode, sessionId]);
  const forgetSession = React.useCallback(
    (id?: string) => {
      forgetStreamSession(id);
      useUiStore.getState().forgetAssistantChatPermissionModes(id);
    },
    [forgetStreamSession],
  );
  // The tool set is the provider's own (Settings → AI), all tools when it has
  // none. It follows the provider, not the chat, and is never derived from
  // the model.
  const providerToolProfiles = useUiStore((state) => state.assistantProviderToolProfiles);
  const toolProfileFor = React.useCallback(
    (provider: string | undefined): AiToolProfile =>
      (provider ? providerToolProfiles[provider] : undefined) ??
      DEFAULT_ASSISTANT_TOOL_PROFILE,
    [providerToolProfiles],
  );
  // Text placed in the composer from elsewhere replaces any parked handoff.
  const setAssistantDraft = useAssistantDraftStore((state) => state.replaceDraft);
  const [queuedPrompts, setQueuedPrompts] = React.useState<QueuedPrompt[]>([]);
  const [incognito, setIncognito] = React.useState(false);
  // The chat's files in the order attached; the daemon names them file-1,
  // file-2, ... by this order. The ref answers synchronously (limits, and
  // a turn sent in the same tick as a file is staged).
  const [attachments, setAttachmentsState] = React.useState<StagedAttachment[]>([]);
  const attachmentsRef = React.useRef<StagedAttachment[]>([]);
  const setAttachments = React.useCallback((next: StagedAttachment[]) => {
    attachmentsRef.current = next;
    setAttachmentsState(next);
  }, []);
  // Pasted or dropped files on their way to the daemon, shown as cards
  // meanwhile; sending waits for them.
  const [uploads, setUploads] = React.useState<Array<AssistantAttachment & { key: string }>>([]);
  const uploadsRef = React.useRef(0);
  // Files a sent message already carried. Later turns keep using them
  // without showing them again.
  const shownTokensRef = React.useRef<ReadonlySet<string>>(new Set());
  const [shownTokens, setShownTokensState] = React.useState<ReadonlySet<string>>(new Set());
  const setShownTokens = React.useCallback((next: ReadonlySet<string>) => {
    shownTokensRef.current = next;
    setShownTokensState(next);
  }, []);
  // "Ask about this" pin. It belongs to the book it was taken in, so opening
  // another book (a switch, or a new book) drops it. Sends check the pin's
  // book synchronously (`livePin`); the effect below only removes the chip
  // once the change has rendered.
  const [pin, setPin] = React.useState<Pin | null>(null);
  const focus =
    pin && sameBook(pin.book, { daemonSession, scope: bookScope }) ? pin.focus : null;
  // The pin to send now: none when it was taken in a book that is no longer
  // the open one, even when that change lands in the same render as the send.
  const livePin = React.useCallback(
    (candidate: Pin | null): AssistantFocus | null =>
      candidate && sameBook(candidate.book, bookNow()) ? candidate.focus : null,
    [bookNow],
  );
  // Read through a ref, so the effect below runs for a book switch only and
  // a language change (a new `t`) never clears the pin.
  const tRef = React.useRef(t);
  React.useEffect(() => {
    tRef.current = t;
  }, [t]);
  React.useEffect(() => {
    const now = bookNow();
    setPin((current) => (current && !sameBook(current.book, now) ? null : current));
    // A handoff parked in the composer by another book goes with it (the send
    // path checks again, in case a send races this effect).
    const drafts = useAssistantDraftStore.getState();
    if (drafts.handoff && !sameBook(drafts.handoff.book, now)) {
      drafts.replaceDraft("");
      noticeBookSwitched(tRef.current, { returned: false, cleared: true });
    }
  }, [bookNow, bookScope, daemonSession]);
  // Set by branch/edit; consumed on the next send so the daemon persists the
  // seeded prefix for that fork only. A bare detached conversation (history
  // toggled, session deleted) never carries it, so its prior turns are not
  // backfilled into a new session.
  const seedHistoryPendingRef = React.useRef(false);
  // The book the conversation belongs to: bound by its first turn or the
  // first file attached to it, null while it is fresh.
  const conversationBookRef = React.useRef<BookStamp | null>(null);

  // Runs one chat turn against an explicit conversation base + session, so
  // callers that rewind history (edit) can regenerate atomically without
  // waiting for `messages`/`sessionId` state to settle first.
  const runTurn = React.useCallback(
    (
      prompt: string,
      baseMessages: AiChatMessage[],
      activeSession: string | null,
      evidence?: { request: EvidenceRequest; attachment?: StagedAttachment },
      // What a submitted question was asked under: the pin then (dropped if
      // its book is gone) and the page it was asked on, or a handoff's own
      // page context. Omitted means the pin and the page now.
      asked?: Pick<QueuedPrompt, "pin" | "context" | "handoff" | "book">,
    ): boolean => {
      if (!selection?.model) return false;
      const generated = Boolean(evidence || asked?.handoff);
      const daemonSessionNow = useUiStore.getState().daemonSession;
      // The book this turn was asked in, never merely the one open when it is
      // sent: a queued question's or handoff's stamp, the evidence request's
      // own book, or the open book at this call (the user's edit confirm).
      const turnBook: BookStamp | null = evidence
        ? {
            daemonSession: daemonSessionNow,
            scope: { workspace_id: evidence.request.workspace_id, profile_id: evidence.request.profile_id },
          }
        : asked
          ? asked.book
          : captureBook();
      if (!turnBook) {
        refuseBookLoading(tRef.current, generated ? undefined : prompt);
        return false;
      }
      const expectedScope = turnBook.scope;
      const baseConversation = baseMessages.filter((message) => message.role !== "system");
      // A file the evidence request just staged may not be in this render's
      // list yet, so it is added here.
      const chatFiles = attachmentsRef.current;
      const sentFiles =
        evidence?.attachment && !chatFiles.some((file) => file.token === evidence.attachment?.token)
          ? [...chatFiles, evidence.attachment]
          : chatFiles;
      // Backstop to the fresh conversation another book starts (which may not
      // have rendered yet): the turn, its transcript, saved session and
      // attached file all belong to the open book, or nothing is sent.
      const bound = conversationBookRef.current;
      const carried =
        baseConversation.length > 0 || activeSession !== null || sentFiles.length > 0;
      if (
        !sameBook(turnBook, bookNow()) ||
        (carried && (!bound || !sameBook(bound, turnBook))) ||
        sentFiles.some((file) => !sameBook(file.book, turnBook))
      ) {
        refuseInOtherBook(tRef.current, prompt, generated);
        return false;
      }
      if (!bound) conversationBookRef.current = turnBook;
      turnStampRef.current = { generation: generationRef.current,
        daemonSession: daemonSessionNow, promptRevision: promptRevisionRef.current };
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
      // The message that first carries a file shows it, and tells the daemon
      // so: a provider that resumes its own session saw earlier ones there.
      const shownBefore = shownTokensRef.current;
      const newFiles = sentFiles.filter((file) => !shownBefore.has(file.token));
      if (newFiles.length) {
        setShownTokens(new Set([...shownBefore, ...newFiles.map((file) => file.token)]));
      }
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
          toolProfile: toolProfileFor(selection.provider),
          systemPromptKind: "kassiber",
          sessionId: activeSession,
          persist: incognito && activeSession === null ? false : "auto",
          seedHistory,
          ...(seedNotices.length ? { seedNotices } : {}),
          // Tokens only. `label` is the user's own words about a file (the
          // CLI's --file-context); sending the filename there would have the
          // daemon tell the model "the user describes it as: export.csv", and
          // the daemon already names each file from its staging grant.
          attachments: sentFiles.length
            ? sentFiles.map((file) =>
                newFiles.includes(file) ? { token: file.token, new: true } : { token: file.token },
              )
            : undefined,
          expectedScope,
          // Read at send time, so a queued question runs in the mode chosen
          // while it waited.
          permissionMode: permissionModeRef.current,
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
        // A page's or an evidence request's own turn shows no files; the
        // user's next message does not show them again either.
        !evidence && newFiles.length
          ? newFiles.map((file) => ({
              filename: file.filename,
              kind: file.kind,
              sizeBytes: file.sizeBytes,
              previewUrl: file.previewUrl,
            }))
          : undefined,
      )).then((outcome) => {
        // Files whose staging grant ended: the turn went on without them, and
        // the chat drops them so later turns do not offer them either.
        const expired = outcome?.expiredAttachments;
        if (expired?.length) {
          const gone = new Set(expired);
          const names = attachmentsRef.current
            .filter((file) => gone.has(file.token))
            .map((file) => file.filename);
          setAttachments(attachmentsRef.current.filter((file) => !gone.has(file.token)));
          useUiStore.getState().addNotification({
            title: tRef.current("composer.attachmentExpired.title", { count: expired.length }),
            body: tRef.current("composer.attachmentExpired.body", {
              count: expired.length,
              filenames: names.join(", "),
            }),
            tone: "info",
            dedupeKey: "assistant-attachment-expired",
          });
        }
        // Refused because another book was open when it arrived: the same
        // rule as a client-side mismatch. A handoff or evidence continuation
        // was written from that book's data and is dropped; the user's own
        // text returns to the composer.
        if (!outcome?.refused) return;
        if (newFiles.length) {
          const unsent = new Set(newFiles.map((file) => file.token));
          setShownTokens(new Set([...shownTokensRef.current].filter((token) => !unsent.has(token))));
        }
        refuseInOtherBook(tRef.current, prompt, generated);
      });
      return true;
    },
    [bookNow, captureBook, fastMode, livePin, pin, incognito, screenContext, selection, send, setAttachments, setShownTokens, thinkingEffort, toolProfileFor],
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

  // An empty conversation in the book open now. The Incognito choice stays;
  // the permission mode returns to the Settings default, like a new chat.
  const startFreshConversation = React.useCallback(() => {
    clearEvidence();
    seedHistoryPendingRef.current = false;
    // The grants belong to the conversation that asked for them; a new chat
    // must not silently keep analyzing the previous chat's files.
    setAttachments([]);
    setUploads([]);
    setShownTokens(new Set());
    reset();
    const fresh = useUiStore.getState().assistantDefaultPermissionMode;
    permissionModeRef.current = fresh;
    setPermissionModeState(fresh);
    conversationBookRef.current = null;
  }, [clearEvidence, reset, setAttachments, setShownTokens]);

  // A conversation belongs to the book it started in. When another book
  // opens (a new daemon session, or other book ids before the session counter
  // has moved), start a fresh one; the old one stays in that book's history
  // when history is on. A file attached there goes too, like a parked
  // handoff, and a notice says so. Queued questions follow their own book
  // rule below.
  React.useEffect(() => {
    const bound = conversationBookRef.current;
    if (!bound || sameBook(bound, { daemonSession, scope: bookScope })) return;
    const hadFile = attachments.length > 0;
    startFreshConversation();
    if (hadFile) {
      useUiStore.getState().addNotification({
        title: tRef.current("queue.bookSwitched.title"),
        body: tRef.current("queue.bookSwitched.bodyAttachment"),
        tone: "warning",
        dedupeKey: "assistant-book-switched-attachment",
      });
    }
  }, [attachments.length, bookScope, daemonSession, startFreshConversation]);

  // A file attached to a fresh conversation binds it to the file's book. A
  // file already attached is not listed twice; past the limit, none is added.
  const stageAttachment = React.useCallback((file: StagedAttachment) => {
    const bound = conversationBookRef.current;
    if (bound && !sameBook(bound, file.book)) return false;
    const current = attachmentsRef.current;
    if (current.some((entry) => entry.token === file.token)) return true;
    if (current.length >= MAX_CHAT_ATTACHMENTS) return false;
    conversationBookRef.current = bound ?? file.book;
    setAttachments([...current, file]);
    return true;
  }, [setAttachments]);

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
    // A file the evidence request asked for belongs to that request's book.
    const staged: StagedAttachment | undefined = nextAttachment && {
      ...nextAttachment,
      book: {
        daemonSession: pending.origin.daemonSession,
        scope: { workspace_id: pending.request.workspace_id, profile_id: pending.request.profile_id },
      },
    };
    if (staged && !stageAttachment(staged)) return;
    pending.outcome = outcome;
    pending.attachment = staged;
    setEvidenceDialog(null);
    setEvidenceRequests((old) => ({ ...old, [pending.request.request_id]: { status: outcome === "partial" ? "partial" : "received" } }));
    const latest = latestRef.current;
    if (canAutoContinueEvidence(pending.origin, stamp(), latest.isStreaming || !latest.hasModel,
      Boolean(useAssistantDraftStore.getState().draft.trim()), latest.queued)) {
      continueEvidenceRequest(pending.request);
    }
  }, [continueEvidenceRequest, currentEvidence, stageAttachment, stamp]);

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
    (item: QueuedPrompt) =>
      runTurn(item.prompt, messages, sessionId, undefined, {
        pin: item.pin,
        context: item.context,
        handoff: item.handoff,
        book: item.book,
      }),
    [messages, runTurn, sessionId],
  );

  const sendPrompt = React.useCallback(
    // Whether the prompt was taken (sent, queued or parked). A refusal made
    // here returns false, so the composer keeps the user's text: the refusal
    // runs while the submitted text is still in the draft, which the
    // composer would otherwise clear right after.
    (prompt: string, explicitHandoff?: AssistantHandoff): boolean => {
      const trimmed = prompt.trim();
      if (!trimmed) return false;
      const now = bookNow();
      // A page's request made in a book that is no longer open is not taken
      // here: its text was written from that book's data.
      if (explicitHandoff && !sameBook(explicitHandoff.book, now)) {
        noticeBookSwitched(t, { returned: false, cleared: true });
        return false;
      }
      if (!selection?.model) {
        // No model yet: a page's request waits in the composer, without the
        // pin, with the page and the book it was made in.
        if (!explicitHandoff) return false;
        setPin(null);
        const page = explicitHandoff.context ?? currentPageScreenContext();
        const drafts = useAssistantDraftStore.getState();
        if (page) drafts.parkHandoff(prompt, { context: page, book: explicitHandoff.book });
        else drafts.replaceDraft(prompt);
        return true;
      }
      let handoff = explicitHandoff;
      // A handoff parked in the composer is sent as that handoff, naming the
      // book it was parked in. One parked in another book is not sent at
      // all, and its text, which carries that book's data, goes too.
      if (!handoff) {
        const parked = useAssistantDraftStore.getState().takeHandoff();
        if (parked && !sameBook(parked.book, now)) {
          useAssistantDraftStore.getState().replaceDraft("");
          noticeBookSwitched(t, { returned: false, cleared: true });
          return false;
        }
        if (parked) handoff = { context: parked.context, book: parked.book };
      }
      // A plain prompt belongs to the book open at this call, the user's
      // submit; it is not taken before that book's ids are known.
      const book = handoff ? handoff.book : captureBook();
      if (!book) {
        refuseBookLoading(t, trimmed);
        return false;
      }
      promptRevisionRef.current += 1;
      // Captured now, so a question that waits in the queue still runs with
      // the page and the book it was asked in. A page's handoff runs with the
      // page it came from and replaces the pin rather than being narrowed by it.
      const item: QueuedPrompt = {
        prompt: trimmed,
        pin: handoff ? null : pin,
        context: handoff?.context ?? currentAssistantScreenContext(screenContext),
        handoff: Boolean(handoff),
        book,
      };
      if (handoff) setPin(null);
      // Also held while the effort is not yet seeded for this book; the queue
      // sends it once it is.
      if (isStreaming || !ready) {
        setQueuedPrompts((current) => [...current, item]);
        return true;
      }
      return dispatchPrompt(item);
    },
    [bookNow, captureBook, dispatchPrompt, ready, pin, isStreaming, screenContext, selection, t],
  );

  React.useEffect(() => {
    if (isStreaming || queuedPrompts.length === 0 || !ready) return;
    if (!selection?.model) return;
    // Checked here, as it is sent, against the book open now: a request
    // queued in another book (switched to, or newly created) never runs in
    // this one (noticeBookSwitched). The daemon checks the book it names too.
    const now = bookNow();
    const belongs = (item: QueuedPrompt) => sameBook(item.book, now);
    const stale = queuedPrompts.filter((item) => !belongs(item));
    if (stale.length > 0) {
      setQueuedPrompts((current) => current.filter(belongs));
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
  }, [bookNow, dispatchPrompt, ready, isStreaming, queuedPrompts, selection, t]);

  const queuedPromptTexts = React.useMemo(
    () => queuedPrompts.map((item) => item.prompt),
    [queuedPrompts],
  );

  const typedSendConsent = React.useCallback(
    (callId: string, decision: AiToolConsentDecision) => sendConsent(callId, decision),
    [sendConsent],
  );

  const choosePermissionMode = React.useCallback(
    (mode: AiPermissionMode) => {
      if (mode === permissionModeRef.current) return;
      permissionModeRef.current = mode;
      setPermissionModeState(mode);
      // A running answer switches too; without one the next turn names it.
      if (!latestRef.current.isStreaming) return;
      void changePermissionMode(mode).then((outcome) => {
        if (!outcome.error) return;
        useUiStore.getState().addNotification({
          title: tRef.current("composer.permission.notApplied.title"),
          body: tRef.current("composer.permission.notApplied.body"),
          tone: "warning",
          dedupeKey: "assistant-permission-mode-not-applied",
        });
      });
    },
    [changePermissionMode],
  );

  const clearChat = React.useCallback(() => {
    setQueuedPrompts([]);
    setPin(null);
    startFreshConversation();
  }, [startFreshConversation]);

  const askAbout = React.useCallback(
    (nextFocus: AssistantFocus, prompt: string) => {
      // Pinned to the book open at the click; not taken before its ids load.
      const book = captureBook();
      if (!book) {
        refuseBookLoading(tRef.current);
        return;
      }
      setPin({ focus: nextFocus, book });
      // A draft, not a send: the user reviews the question (and which model it
      // goes to) before anything leaves the device.
      setAssistantDraft(prompt);
      useUiStore.getState().openAssistantPanel({ focusComposer: true });
    },
    [captureBook, setAssistantDraft],
  );
  const clearFocus = React.useCallback(() => setPin(null), []);

  const notifyAttachment = React.useCallback(
    (key: "attachmentLimit" | "attachmentUnsupported" | "attachmentTooLarge", filename = "") => {
      useUiStore.getState().addNotification({
        title: tRef.current(`composer.${key}.title`, { filename, max: MAX_CHAT_ATTACHMENTS }),
        body: tRef.current(`composer.${key}.body`, { filename, max: MAX_CHAT_ATTACHMENTS }),
        tone: key === "attachmentLimit" ? "info" : "warning",
        dedupeKey: `assistant-${key}`,
      });
    },
    [],
  );

  const notifyAttachmentFailed = React.useCallback((error: unknown) => {
    if (error instanceof ChatAttachmentError) {
      if (error.code === "chat_attachment_unsupported") return notifyAttachment("attachmentUnsupported");
      if (error.code === "chat_attachment_too_large") return notifyAttachment("attachmentTooLarge", error.message);
    }
    useUiStore.getState().addNotification({
      title: tRef.current("composer.attachmentFailed"),
      body: error instanceof Error ? error.message : String(error),
      tone: "warning",
      dedupeKey: "assistant-attachment-failed",
    });
  }, [notifyAttachment]);

  // Room for more files: those listed and those still on their way count.
  const attachmentRoom = React.useCallback(
    () => MAX_CHAT_ATTACHMENTS - attachmentsRef.current.length - uploadsRef.current,
    [],
  );

  const attachFile = React.useCallback(async () => {
    // The file belongs to the book open at the click, captured before the
    // picker; the daemon stages it for that book only.
    const book = captureBook();
    if (!book) {
      refuseBookLoading(tRef.current);
      return;
    }
    if (attachmentRoom() <= 0) {
      notifyAttachment("attachmentLimit");
      return;
    }
    const origin = stamp();
    const selected = await pickChatAttachmentSource({ expected_scope: book.scope });
    if (!sameHandoffContext(origin, stamp()) || !sameBook(book, bookNow())) return;
    if (!selected) return; // cancelled, or no picker in this runtime
    const added = stageAttachment({
      token: selected.document_token,
      filename: selected.source.filename,
      kind: selected.source.kind,
      sizeBytes: selected.source.size_bytes,
      previewUrl: chatThumbnail(selected.source.thumbnail),
      book,
    });
    if (!added) notifyAttachment("attachmentLimit", selected.source.filename);
  }, [attachmentRoom, bookNow, captureBook, notifyAttachment, stageAttachment, stamp]);

  // Pasted files, or files dropped in the browser preview: the bytes go to
  // the daemon, which keeps them in private temporary files and returns
  // tokens like the picker's. They are sent one after another, each shown
  // as a card meanwhile.
  const attachFiles = React.useCallback(async (files: File[]) => {
    const book = captureBook();
    if (!book) {
      refuseBookLoading(tRef.current);
      return;
    }
    const supported = files.filter((candidate) => isChatAttachmentName(candidate.name));
    if (supported.length < files.length) notifyAttachment("attachmentUnsupported");
    const fitting = supported.filter((file) => {
      if (file.size <= MAX_CHAT_UPLOAD_BYTES) return true;
      notifyAttachment("attachmentTooLarge", file.name);
      return false;
    });
    const room = Math.max(0, attachmentRoom());
    if (fitting.length > room) notifyAttachment("attachmentLimit", fitting[room]?.name ?? "");
    const taken = fitting.slice(0, room);
    if (!taken.length) return;
    const origin = stamp();
    const placeholders = await Promise.all(
      taken.map(async (file, index) => ({
        key: `upload-${origin.generation}-${Date.now()}-${index}`,
        token: "",
        filename: file.name,
        kind: chatAttachmentKind(file.name) ?? undefined,
        sizeBytes: file.size,
        previewUrl: (await chatImageThumbnail(file)) ?? undefined,
      })),
    );
    uploadsRef.current += placeholders.length;
    setUploads((current) => [...current, ...placeholders]);
    try {
      for (const [index, file] of taken.entries()) {
        const placeholder = placeholders[index];
        try {
          const selected = await uploadChatAttachment(file, book.scope);
          if (!sameHandoffContext(origin, stamp()) || !sameBook(book, bookNow())) return;
          const added = stageAttachment({
            token: selected.document_token,
            filename: selected.source.filename,
            kind: selected.source.kind,
            sizeBytes: selected.source.size_bytes,
            previewUrl: placeholder.previewUrl,
            book,
          });
          if (!added) notifyAttachment("attachmentLimit", selected.source.filename);
        } catch (error) {
          if (sameHandoffContext(origin, stamp())) notifyAttachmentFailed(error);
        } finally {
          uploadsRef.current -= 1;
          setUploads((current) => current.filter((entry) => entry.key !== placeholder.key));
        }
      }
    } finally {
      // A switch to another chat or book stopped the loop early.
      const left = new Set(placeholders.map((entry) => entry.key));
      setUploads((current) => {
        const kept = current.filter((entry) => !left.has(entry.key));
        uploadsRef.current = kept.length;
        return kept;
      });
    }
  }, [attachmentRoom, bookNow, captureBook, notifyAttachment, notifyAttachmentFailed, stageAttachment, stamp]);

  // Files dropped on the desktop window: the shell kept the paths of that
  // drop and stages them; the window never names one.
  const attachDroppedFiles = React.useCallback(async () => {
    const book = captureBook();
    if (!book) {
      refuseBookLoading(tRef.current);
      return;
    }
    const origin = stamp();
    let dropped: DroppedChatAttachments | null;
    try {
      dropped = await stageDroppedChatAttachments(book.scope);
    } catch (error) {
      notifyAttachmentFailed(error);
      return;
    }
    if (!dropped || !sameHandoffContext(origin, stamp()) || !sameBook(book, bookNow())) return;
    if (dropped.unsupportedCount > 0) notifyAttachment("attachmentUnsupported");
    let refused = dropped.droppedCount - dropped.unsupportedCount - dropped.staged.length;
    for (const selected of dropped.staged) {
      const added = stageAttachment({
        token: selected.document_token,
        filename: selected.source.filename,
        kind: selected.source.kind,
        sizeBytes: selected.source.size_bytes,
        previewUrl: chatThumbnail(selected.source.thumbnail),
        book,
      });
      if (!added) refused += 1;
    }
    if (refused > 0) notifyAttachment("attachmentLimit");
  }, [bookNow, captureBook, notifyAttachment, notifyAttachmentFailed, stageAttachment, stamp]);

  // A file already sent stays with the chat until removed here; a later
  // turn no longer offers it to the model.
  const removeAttachment = React.useCallback(
    (key: string) => setAttachments(attachmentsRef.current.filter((file) => file.token !== key)),
    [setAttachments],
  );

  const resumeSession = React.useCallback(
    async (targetSessionId: string) => {
      if (isStreaming) return;
      // The saved chat belongs to the book open at the click; the daemon
      // refuses the read if another one is open when it arrives.
      const book = captureBook();
      if (!book) {
        refuseBookLoading(tRef.current);
        return;
      }
      clearEvidence();
      const resumeGeneration = generationRef.current;
      const envelope = await getTransport().invoke<StoredSessionShape>({
        kind: "ui.chat.sessions.get",
        request_id: makeDaemonRequestId(),
        args: { session_id: targetSessionId, expected_scope: book.scope },
      });
      if (envelope.kind === "error" || envelope.error) {
        throw new Error(
          envelope.error?.message ?? "Could not load the chat session",
        );
      }
      // Another book opened meanwhile and started a fresh conversation.
      if (generationRef.current !== resumeGeneration || !sameBook(book, bookNow())) {
        return;
      }
      const entries = storedChatEntries(envelope.data?.messages ?? []);
      setQueuedPrompts([]);
      setIncognito(false);
      // The grants belong to the conversation that asked for them. Loading a
      // different saved chat must not carry the previous chat's files into it —
      // the next turn would analyze files this conversation never mentioned.
      setAttachments([]);
      setShownTokens(new Set());
      setPin(null);
      // Drop any half-typed draft before binding the resumed (persisted)
      // session — otherwise text typed while Incognito would ride into the
      // loaded chat and be stored on the next submit.
      setAssistantDraft("");
      seedHistoryPendingRef.current = false;
      const loadedSessionId = envelope.data?.id ?? targetSessionId;
      // The mode this chat last used, or the default for one never seen here.
      const ui = useUiStore.getState();
      const resumedMode =
        ui.assistantChatPermissionModes[loadedSessionId] ??
        ui.assistantDefaultPermissionMode;
      permissionModeRef.current = resumedMode;
      setPermissionModeState(resumedMode);
      loadConversation(entries, loadedSessionId);
      conversationBookRef.current = book;
    },
    [bookNow, captureBook, clearEvidence, isStreaming, loadConversation, setAssistantDraft, setAttachments, setShownTokens],
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
        // and the page that are active now, in the book open at the confirm.
        const book = captureBook();
        if (!book) {
          refuseBookLoading(tRef.current, trimmed);
          return;
        }
        setQueuedPrompts([
          {
            prompt: trimmed,
            pin,
            context: currentAssistantScreenContext(screenContext),
            handoff: false,
            book,
          },
        ]);
        return;
      }
      runTurn(trimmed, priorMessages, null);
    },
    [
      captureBook,
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

  const chatFiles = React.useMemo<AssistantChatFile[]>(
    () => [
      // The book stamp stays here; the composer gets what it shows.
      ...attachments.map((file) => ({
        token: file.token,
        filename: file.filename,
        kind: file.kind,
        sizeBytes: file.sizeBytes,
        previewUrl: file.previewUrl,
        key: file.token,
        state: shownTokens.has(file.token) ? ("sent" as const) : ("ready" as const),
      })),
      ...uploads.map((upload) => ({ ...upload, state: "uploading" as const })),
    ],
    [attachments, shownTokens, uploads],
  );

  const value = React.useMemo<AssistantSessionContextValue>(
    () => ({
      evidenceRequests, openEvidenceRequest, continueEvidenceRequest,
      messages,
      isStreaming,
      error,
      pendingConsents,
      queuedPrompts: queuedPromptTexts,
      selection,
      thinkingEffort,
      fastMode,
      returnPath: screenContext.route,
      sessionId,
      incognito,
      attachments: chatFiles,
      setSelection,
      setThinkingEffort: chooseThinkingEffort,
      // A file still on its way would miss the message.
      sendBlockedReason: sendBlockedReason ?? (uploads.length ? t("composer.attachmentUploading") : null),
      setFastMode,
      setIncognito,
      attachFile,
      attachFiles,
      attachDroppedFiles,
      removeAttachment,
      sendPrompt,
      sendConsent: typedSendConsent,
      permissionMode,
      setPermissionMode: choosePermissionMode,
      abort,
      reset: clearChat,
      resumeSession,
      branchFromMessage,
      editUserMessage,
      forgetSession,
      focus,
      askAbout,
      clearFocus,
      captureBook,
    }),
    [
      askAbout,
      captureBook,
      clearFocus,
      focus,
      evidenceRequests, openEvidenceRequest, continueEvidenceRequest,
      abort,
      attachFile,
      attachFiles,
      attachDroppedFiles,
      branchFromMessage,
      chatFiles,
      chooseThinkingEffort,
      removeAttachment,
      choosePermissionMode,
      editUserMessage,
      clearChat,
      error,
      fastMode,
      forgetSession,
      incognito,
      isStreaming,
      messages,
      pendingConsents,
      permissionMode,
      queuedPromptTexts,
      resumeSession,
      screenContext.route,
      sessionId,
      setSelection,
      selection,
      sendBlockedReason,
      sendPrompt,
      t,
      uploads.length,
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
