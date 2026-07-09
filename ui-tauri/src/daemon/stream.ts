/**
 * `useAiChatStream` — drives one streaming `ai.chat` call against the
 * active daemon transport.
 *
 * Returns a small imperative API plus reducer state. Each delta from the
 * daemon's `ai.chat.delta` records is split into `content` / `thinking`
 * channels via `ThinkParser`, so the reasoning pane and answer pane can
 * render independently as tokens arrive.
 *
 * Stop sends a cooperative `ai.chat.cancel` daemon request for the active
 * stream request_id, while the UI still suppresses late records locally.
 */

import * as React from "react";

import type {
  DaemonEnvelope,
  DaemonStreamRecord,
} from "./transport";
import { getTransport, makeDaemonRequestId } from "./transport";
import { ThinkParser } from "@/lib/thinkParser";
import { useUiStore } from "@/store/ui";

/** One model-round reasoning trace within an assistant turn. */
export interface AiChatThinkingSegment {
  id: string;
  content: string;
}

export interface AiChatMessage {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  /**
   * Joined reasoning text for the turn (legacy / convenience). Prefer
   * `thinkingSegments` when rendering — each segment is one provider
   * completion round (Ollama/oMLX emit a fresh reasoning stream per call).
   */
  thinking?: string;
  /** Per model-round reasoning panes for this assistant turn. */
  thinkingSegments?: AiChatThinkingSegment[];
  activityLabel?: string;
  toolCalls?: AiChatToolCall[];
  status: "pending" | "streaming" | "done" | "error" | "cancelled";
  errorCode?: string;
  errorMessage?: string;
  finishReason?: string | null;
  provider?: string;
  model?: string;
  provenance?: AiAnswerProvenance;
}

export interface AiAnswerProvenance {
  generated_at?: string;
  provider?: string;
  model?: string;
  tools_used?: string[];
  active_transactions?: number | null;
  quarantines?: number | null;
  missing_price_transactions?: number | null;
  journals_processed_at?: string | null;
  auto_journal_processed?: boolean;
  auto_sync_attempted?: boolean;
  auto_sync_ok?: boolean | null;
}

export type AiToolCallStatus =
  | "pending"
  | "awaiting_consent"
  | "running"
  | "done"
  | "denied"
  | "error";

export type AiToolConsentDecision = "allow_once" | "allow_session" | "deny";

export interface AiChatToolCall {
  callId: string;
  name: string;
  arguments: Record<string, unknown>;
  kindClass: "read_only" | "mutating" | "unknown";
  needsConsent: boolean;
  status: AiToolCallStatus;
  result?: unknown;
  reason?: string;
}

export interface AiChatRequest {
  provider?: string;
  model: string;
  messages: { role: AiChatMessage["role"] | "tool"; content: string }[];
  options?: Record<string, unknown>;
  toolsEnabled?: boolean;
  toolLoopMaxIterations?: number;
  systemPromptKind?: "kassiber" | "raw" | null;
  systemPrompt?: string;
  /** Append this exchange to an existing persisted session. */
  sessionId?: string | null;
  /**
   * Persistence intent: "auto" follows the stored ai_chat_history policy
   * (off / on / encrypted-only), false skips persistence (incognito).
   * Absent means no persistence, which keeps legacy callers unchanged.
   */
  persist?: boolean | "auto";
  /**
   * Explicit branch/edit fork: persist the seeded prefix (prior on-screen
   * turns) when this creates a new session. Only set for the first turn after
   * a fork — a null session id alone must not backfill detached history.
   */
  seedHistory?: boolean;
}

/** One stored exchange row from ui.chat.sessions.get. */
export interface StoredChatEntry {
  role: "user" | "assistant";
  content: string;
}

export function buildAiChatStreamArgs(
  request: AiChatRequest,
): Record<string, unknown> {
  return {
    provider: request.provider,
    model: request.model,
    messages: request.messages,
    options: request.options,
    tools_enabled: request.toolsEnabled,
    tool_loop_max_iterations: request.toolLoopMaxIterations,
    system_prompt_kind: request.systemPromptKind,
    system_prompt: request.systemPrompt,
    session_id: request.sessionId ?? undefined,
    persist: request.persist,
    seed_history: request.seedHistory ? true : undefined,
  };
}

export interface AiToolConsentRequest {
  targetRequestId: string;
  callId: string;
  name: string;
  summary: string;
  argumentsPreview: Record<string, unknown>;
}

export interface AiChatDeltaShape {
  delta?: {
    role?: AiChatMessage["role"] | "tool";
    content?: string;
    /**
     * Structured reasoning channel emitted by OpenAI o1/o3-style models
     * and by Ollama's OpenAI-compat shim for Qwen3 / Gemma reasoning
     * builds. Distinct from inline `<think>...</think>` tags inside
     * `content`, which `ThinkParser` handles. Both flow into the same
     * UI thinking pane.
     */
    reasoning?: string;
  };
}

export interface AiChatStatusShape {
  phase?: string;
  label?: string;
}

interface AiChatTerminalShape {
  provider?: string;
  model?: string;
  finish_reason?: string | null;
  provenance?: AiAnswerProvenance;
  session_id?: string | null;
}

export interface AiChatToolCallShape {
  call_id: string;
  name: string;
  arguments?: Record<string, unknown>;
  kind_class?: "read_only" | "mutating" | "unknown";
  needs_consent?: boolean;
}

export interface AiChatToolResultShape {
  call_id: string;
  ok?: boolean;
  reason?: string;
  envelope?: unknown;
  message?: string;
}

export interface AiChatToolConsentRequiredShape {
  call_id: string;
  name: string;
  summary?: string;
  arguments_preview?: Record<string, unknown>;
}

interface AiToolConsentResponseShape {
  recorded?: boolean;
  reason?: string;
}

type AiChatStreamRecordData =
  | AiChatStatusShape
  | AiChatDeltaShape
  | AiChatToolCallShape
  | AiChatToolResultShape
  | AiChatToolConsentRequiredShape;

export interface UseAiChatStreamResult {
  messages: AiChatMessage[];
  isStreaming: boolean;
  error: { code: string; message: string } | null;
  pendingConsent: AiToolConsentRequest | null;
  /** Persisted session backing this conversation, if any. */
  sessionId: string | null;
  send: (request: AiChatRequest, userMessageContent: string) => Promise<void>;
  sendConsent: (decision: AiToolConsentDecision) => Promise<void>;
  abort: () => void;
  reset: () => void;
  /** Replace the conversation with a stored session's messages. */
  loadConversation: (
    entries: StoredChatEntry[],
    sessionId: string | null,
  ) => void;
  /** Drop the session binding (all sessions, or only when it matches id). */
  forgetSession: (id?: string) => void;
}

function makeId(): string {
  if (
    typeof globalThis.crypto !== "undefined" &&
    typeof globalThis.crypto.randomUUID === "function"
  ) {
    return globalThis.crypto.randomUUID();
  }
  return `msg-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

/** Non-empty reasoning segments for rendering. */
export function visibleThinkingSegments(
  message: AiChatMessage,
): AiChatThinkingSegment[] {
  const segments = message.thinkingSegments;
  if (segments && segments.length > 0) {
    return segments.filter((segment) => segment.content.length > 0);
  }
  const legacy = message.thinking?.trim();
  if (!legacy) return [];
  return [{ id: `${message.id}-thinking`, content: message.thinking! }];
}

/**
 * Open a new reasoning segment for the next provider completion round.
 * Skips when the current segment is still empty so repeated status
 * records do not create blank panes.
 */
export function beginThinkingSegment(current: AiChatMessage): AiChatMessage {
  const segments = current.thinkingSegments ?? [];
  const last = segments[segments.length - 1];
  if (last && last.content.length === 0) {
    return current;
  }
  return {
    ...current,
    thinkingSegments: [...segments, { id: makeId(), content: "" }],
  };
}

function appendThinkingToMessage(
  current: AiChatMessage,
  thinkingAdd: string,
): AiChatMessage {
  if (!thinkingAdd) return current;
  const segments = [...(current.thinkingSegments ?? [])];
  if (segments.length === 0) {
    segments.push({ id: makeId(), content: thinkingAdd });
  } else {
    const last = segments[segments.length - 1]!;
    segments[segments.length - 1] = {
      ...last,
      content: last.content + thinkingAdd,
    };
  }
  return {
    ...current,
    thinkingSegments: segments,
    thinking: segments
      .map((segment) => segment.content)
      .filter((part) => part.length > 0)
      .join("\n\n"),
  };
}

export function applyAiChatDeltaToMessage(
  current: AiChatMessage,
  record: DaemonStreamRecord<AiChatDeltaShape>,
  parser: ThinkParser,
  aborted: boolean,
): AiChatMessage {
  if (aborted || record.kind !== "ai.chat.delta") return current;
  const content = record.data?.delta?.content;
  const reasoning = record.data?.delta?.reasoning;
  if (!content && !reasoning) return current;

  // `content` may carry inline `<think>...</think>` chunks (DeepSeek-R1,
  // older Qwen builds). `reasoning` is the structured channel
  // (OpenAI o1/o3, Ollama's OpenAI-compat for Qwen3 / Gemma reasoning
  // builds). Both append to the active thinking segment; visible answer
  // comes from the parsed-content channel only.
  let visibleAdd = "";
  let thinkingAdd = "";
  if (content) {
    const split = parser.feed(content);
    visibleAdd = split.content;
    thinkingAdd = split.thinking;
  }
  if (reasoning) {
    thinkingAdd += reasoning;
  }
  if (!visibleAdd && !thinkingAdd) return current;

  let next: AiChatMessage = {
    ...current,
    status: "streaming",
    content: current.content + visibleAdd,
    activityLabel:
      !visibleAdd && thinkingAdd ? "Thinking" : current.activityLabel,
  };
  if (thinkingAdd) {
    next = appendThinkingToMessage(next, thinkingAdd);
  }
  return next;
}

export function aiChatStatusLabel(data: AiChatStatusShape | undefined): string {
  const explicit = data?.label?.trim();
  if (explicit) return explicit;
  switch (data?.phase) {
    case "preparing":
      return "Preparing chat";
    case "connecting":
      return "Connecting";
    case "waiting_for_model":
      return "Loading model";
    case "thinking":
      return "Thinking";
    default:
      return "Generating";
  }
}

function toolResultStatus(data: AiChatToolResultShape): AiToolCallStatus {
  if (data.ok) return "done";
  if (
    data.reason === "tool_not_allowed" ||
    data.reason === "user_denied" ||
    data.reason === "consent_timeout"
  ) {
    return "denied";
  }
  return "error";
}

export function buildToolConsentArgs(
  request: AiToolConsentRequest,
  decision: AiToolConsentDecision,
): Record<string, unknown> {
  return {
    target_request_id: request.targetRequestId,
    call_id: request.callId,
    decision,
  };
}

export function buildChatCancelArgs(
  targetRequestId: string,
): Record<string, unknown> {
  return { target_request_id: targetRequestId };
}

export function applyToolConsentResponseToMessage(
  current: AiChatMessage,
  callId: string,
  decision: AiToolConsentDecision,
  recorded: boolean,
  reason?: string,
): AiChatMessage {
  const nextStatus: AiToolCallStatus = recorded
    ? decision === "deny"
      ? "denied"
      : "running"
    : "error";
  return {
    ...current,
    toolCalls: (current.toolCalls ?? []).map((toolCall) =>
      toolCall.callId === callId
        ? {
            ...toolCall,
            status: nextStatus,
            reason: recorded
              ? decision === "deny"
                ? "user_denied"
                : toolCall.reason
              : reason ?? "not_found",
          }
        : toolCall,
    ),
  };
}

export function applyAiChatStreamRecordToMessage(
  current: AiChatMessage,
  record: DaemonStreamRecord<AiChatStreamRecordData>,
  parser: ThinkParser,
  aborted: boolean,
): AiChatMessage {
  if (aborted) return current;
  if (record.kind === "ai.chat.delta") {
    return applyAiChatDeltaToMessage(
      current,
      record as DaemonStreamRecord<AiChatDeltaShape>,
      parser,
      false,
    );
  }
  if (record.kind === "ai.chat.status") {
    const statusData = record.data as AiChatStatusShape | undefined;
    const phase = statusData?.phase;
    // Each provider completion round starts with waiting_for_model (see
    // `_stream_ai_chat_tool_turn`). Open a fresh reasoning segment so
    // Ollama/oMLX traces stay per-round instead of one continuous blob.
    const withSegment =
      phase === "waiting_for_model" || phase === "thinking"
        ? beginThinkingSegment(current)
        : current;
    return {
      ...withSegment,
      status: "streaming",
      activityLabel: aiChatStatusLabel(statusData),
    };
  }
  if (record.kind === "ai.chat.tool_call") {
    const data = record.data as AiChatToolCallShape | undefined;
    if (!data?.call_id || !data.name) return current;
    const nextToolCall: AiChatToolCall = {
      callId: data.call_id,
      name: data.name,
      arguments: data.arguments ?? {},
      kindClass: data.kind_class ?? "unknown",
      needsConsent: Boolean(data.needs_consent),
      status: data.needs_consent ? "pending" : "running",
    };
    const existing = current.toolCalls ?? [];
    const found = existing.some((toolCall) => toolCall.callId === data.call_id);
    return {
      ...current,
      status: "streaming",
      toolCalls: found
        ? existing.map((toolCall) =>
            toolCall.callId === data.call_id
              ? { ...toolCall, ...nextToolCall }
              : toolCall,
          )
        : [...existing, nextToolCall],
    };
  }
  if (record.kind === "ai.chat.tool_result") {
    const data = record.data as AiChatToolResultShape | undefined;
    if (!data?.call_id) return current;
    const existing = current.toolCalls ?? [];
    const status = toolResultStatus(data);
    const found = existing.some((toolCall) => toolCall.callId === data.call_id);
    const applyResult = (toolCall: AiChatToolCall): AiChatToolCall => ({
      ...toolCall,
      status,
      result: data.envelope ?? data.message ?? null,
      reason: data.reason,
    });
    return {
      ...current,
      status: "streaming",
      toolCalls: found
        ? existing.map((toolCall) =>
            toolCall.callId === data.call_id ? applyResult(toolCall) : toolCall,
          )
        : [
            ...existing,
            applyResult({
              callId: data.call_id,
              name: "Tool",
              arguments: {},
              kindClass: "unknown",
              needsConsent: false,
              status,
            }),
          ],
    };
  }
  if (record.kind === "ai.chat.tool_consent_required") {
    const data = record.data as AiChatToolConsentRequiredShape | undefined;
    if (!data?.call_id || !data.name) return current;
    const existing = current.toolCalls ?? [];
    const found = existing.some((toolCall) => toolCall.callId === data.call_id);
    const nextToolCall: AiChatToolCall = {
      callId: data.call_id,
      name: data.name,
      arguments: data.arguments_preview ?? {},
      kindClass: "mutating",
      needsConsent: true,
      status: "awaiting_consent",
    };
    return {
      ...current,
      status: "streaming",
      toolCalls: found
        ? existing.map((toolCall) =>
            toolCall.callId === data.call_id
              ? { ...toolCall, ...nextToolCall }
              : toolCall,
          )
        : [...existing, nextToolCall],
    };
  }
  return current;
}

export function terminalAiChatStatus(
  finishReason: string | null | undefined,
  aborted: boolean,
): AiChatMessage["status"] {
  return aborted || finishReason === "cancelled" ? "cancelled" : "done";
}

/** React hook driving one assistant thread. */
export function useAiChatStream(): UseAiChatStreamResult {
  const dataMode = useUiStore((state) => state.dataMode);
  const [messages, setMessages] = React.useState<AiChatMessage[]>([]);
  const [isStreaming, setIsStreaming] = React.useState(false);
  const [error, setError] = React.useState<UseAiChatStreamResult["error"]>(
    null,
  );
  const [pendingConsent, setPendingConsent] =
    React.useState<AiToolConsentRequest | null>(null);
  const [sessionId, setSessionId] = React.useState<string | null>(null);
  const abortRef = React.useRef<AbortController | null>(null);
  const parserRef = React.useRef<ThinkParser | null>(null);
  const assistantIdRef = React.useRef<string | null>(null);
  const requestIdRef = React.useRef<string | null>(null);
  const recordQueueRef = React.useRef<
    DaemonStreamRecord<AiChatStreamRecordData>[]
  >([]);
  const flushTimerRef = React.useRef<number | null>(null);
  const dataModeRef = React.useRef(dataMode);
  const mountedRef = React.useRef(true);

  React.useEffect(() => {
    dataModeRef.current = dataMode;
  }, [dataMode]);

  const updateAssistant = React.useCallback(
    (
      mutator: (current: AiChatMessage) => AiChatMessage,
    ) => {
      const id = assistantIdRef.current;
      if (!id) return;
      setMessages((prev) =>
        prev.map((message) => (message.id === id ? mutator(message) : message)),
      );
    },
    [],
  );

  const flushQueuedRecords = React.useCallback(() => {
    if (!mountedRef.current) return;
    if (flushTimerRef.current !== null) {
      window.clearTimeout(flushTimerRef.current);
      flushTimerRef.current = null;
    }
    const records = recordQueueRef.current.splice(0);
    if (records.length === 0) return;

    const parser = parserRef.current ?? new ThinkParser();
    parserRef.current = parser;
    updateAssistant((current) =>
      records.reduce(
        (next, record) =>
          applyAiChatStreamRecordToMessage(
            next,
            record,
            parser,
            abortRef.current?.signal.aborted ?? false,
          ),
        current,
      ),
    );
  }, [updateAssistant]);

  const scheduleRecordFlush = React.useCallback(() => {
    if (flushTimerRef.current !== null) return;
    flushTimerRef.current = window.setTimeout(() => {
      flushTimerRef.current = null;
      flushQueuedRecords();
    }, 32);
  }, [flushQueuedRecords]);

  const onRecord = React.useCallback(
    (record: DaemonStreamRecord<AiChatStreamRecordData>) => {
      if (!mountedRef.current) return;
      if (record.kind === "ai.chat.tool_consent_required") {
        const data = record.data as AiChatToolConsentRequiredShape | undefined;
        const targetRequestId =
          typeof record.request_id === "string"
            ? record.request_id
            : requestIdRef.current;
        if (data?.call_id && data.name && targetRequestId) {
          setPendingConsent({
            targetRequestId,
            callId: data.call_id,
            name: data.name,
            summary: data.summary ?? data.name,
            argumentsPreview: data.arguments_preview ?? {},
          });
        }
      }
      if (record.kind === "ai.chat.tool_result") {
        const data = record.data as AiChatToolResultShape | undefined;
        if (data?.call_id) {
          setPendingConsent((current) =>
            current?.callId === data.call_id ? null : current,
          );
        }
      }
      recordQueueRef.current.push(record);
      scheduleRecordFlush();
    },
    [scheduleRecordFlush],
  );

  const cancelActiveRequest = React.useCallback(() => {
    const requestId = requestIdRef.current;
    abortRef.current?.abort();
    recordQueueRef.current = [];
    if (flushTimerRef.current !== null) {
      window.clearTimeout(flushTimerRef.current);
      flushTimerRef.current = null;
    }
    if (requestId) {
      void getTransport(dataModeRef.current)
        .invoke({
          kind: "ai.chat.cancel",
          request_id: makeDaemonRequestId(),
          args: buildChatCancelArgs(requestId),
        })
        .catch(() => undefined);
    }
  }, []);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      cancelActiveRequest();
    };
  }, [cancelActiveRequest]);

  const send = React.useCallback(
    async (request: AiChatRequest, userMessageContent: string) => {
      if (isStreaming) return;
      setError(null);
      setIsStreaming(true);
      setPendingConsent(null);
      const userId = makeId();
      const assistantId = makeId();
      assistantIdRef.current = assistantId;
      parserRef.current = new ThinkParser();
      setMessages((prev) => [
        ...prev,
        {
          id: userId,
          role: "user",
          content: userMessageContent,
          status: "done",
        },
        {
          id: assistantId,
          role: "assistant",
          content: "",
          thinking: "",
          thinkingSegments: [],
          activityLabel: "Preparing chat",
          status: "pending",
        },
      ]);

      const controller = new AbortController();
      abortRef.current = controller;
      const requestId = makeDaemonRequestId();
      requestIdRef.current = requestId;
      try {
        const transport = getTransport(dataMode);
        const envelope = (await transport.stream<
          AiChatTerminalShape,
          AiChatStreamRecordData
        >(
          {
            kind: "ai.chat",
            request_id: requestId,
            args: buildAiChatStreamArgs(request),
          },
          { onRecord, signal: controller.signal },
        )) as DaemonEnvelope<AiChatTerminalShape>;
        if (!mountedRef.current) return;
        flushQueuedRecords();

        if (envelope.kind === "error" || envelope.error) {
          const code = envelope.error?.code ?? "unknown_error";
          const message = envelope.error?.message ?? "AI chat failed";
          if (code === "not_found") {
            // The persisted session backing this conversation is gone
            // (deleted elsewhere); detach so the next turn starts fresh.
            setSessionId(null);
          }
          setError({ code, message });
          updateAssistant((current) => ({
            ...current,
            status: "error",
            errorCode: code,
            errorMessage: message,
          }));
          return;
        }

        const finishReason = envelope.data?.finish_reason ?? null;
        // Flush any pending tag-prefix bytes still in the parser.
        const parser = parserRef.current;
        if (parser && !controller.signal.aborted) {
          const tail = parser.flush();
          if (tail.content || tail.thinking) {
            updateAssistant((current) => {
              let next: AiChatMessage = {
                ...current,
                content: current.content + tail.content,
              };
              if (tail.thinking) {
                next = appendThinkingToMessage(next, tail.thinking);
              }
              return next;
            });
          }
        }

        if (request.persist !== false) {
          // Incognito turns keep the existing session binding: skipping
          // storage for one exchange must not fork the conversation.
          setSessionId(
            typeof envelope.data?.session_id === "string"
              ? envelope.data.session_id
              : null,
          );
        }
        updateAssistant((current) => ({
          ...current,
          status: terminalAiChatStatus(finishReason, controller.signal.aborted),
          finishReason,
          provider: envelope.data?.provider ?? request.provider,
          model: envelope.data?.model ?? request.model,
          provenance: envelope.data?.provenance,
        }));
      } catch (caught) {
        if (!mountedRef.current) return;
        if (controller.signal.aborted) {
          updateAssistant((current) => ({
            ...current,
            status: "cancelled",
          }));
          return;
        }
        const message =
          caught instanceof Error ? caught.message : String(caught);
        setError({ code: "stream_failed", message });
        updateAssistant((current) => ({
          ...current,
          status: "error",
          errorCode: "stream_failed",
          errorMessage: message,
        }));
      } finally {
        if (flushTimerRef.current !== null) {
          window.clearTimeout(flushTimerRef.current);
          flushTimerRef.current = null;
        }
        recordQueueRef.current = [];
        if (mountedRef.current) {
          setIsStreaming(false);
          setPendingConsent(null);
        }
        abortRef.current = null;
        parserRef.current = null;
        assistantIdRef.current = null;
        requestIdRef.current = null;
      }
    },
    [dataMode, flushQueuedRecords, isStreaming, onRecord, updateAssistant],
  );

  const sendConsent = React.useCallback(
    async (decision: AiToolConsentDecision) => {
      const request = pendingConsent;
      if (!request) return;
      setPendingConsent(null);
      try {
        const envelope =
          await getTransport(dataMode).invoke<AiToolConsentResponseShape>({
            kind: "ai.tool_call.consent",
            request_id: makeDaemonRequestId(),
            args: buildToolConsentArgs(request, decision),
          });
        if (!mountedRef.current) return;
        if (envelope.kind === "error" || envelope.error) {
          setError({
            code: envelope.error?.code ?? "consent_failed",
            message: envelope.error?.message ?? "Could not record tool consent",
          });
          updateAssistant((current) =>
            applyToolConsentResponseToMessage(
              current,
              request.callId,
              decision,
              false,
              envelope.error?.code ?? "consent_failed",
            ),
          );
          return;
        }
        if (envelope.data?.recorded === false) {
          const reason = envelope.data.reason ?? "not_found";
          setError({
            code: "consent_not_recorded",
            message: `Could not record tool consent: ${reason}`,
          });
          updateAssistant((current) =>
            applyToolConsentResponseToMessage(
              current,
              request.callId,
              decision,
              false,
              reason,
            ),
          );
          return;
        }
        updateAssistant((current) =>
          applyToolConsentResponseToMessage(
            current,
            request.callId,
            decision,
            true,
          ),
        );
      } catch (caught) {
        if (!mountedRef.current) return;
        const message =
          caught instanceof Error ? caught.message : String(caught);
        setError({ code: "consent_failed", message });
        updateAssistant((current) =>
          applyToolConsentResponseToMessage(
            current,
            request.callId,
            decision,
            false,
            "consent_failed",
          ),
        );
      }
    },
    [dataMode, pendingConsent, updateAssistant],
  );

  const abort = React.useCallback(() => {
    cancelActiveRequest();
    setPendingConsent(null);
    updateAssistant((current) => ({
      ...current,
      status: "cancelled",
    }));
  }, [cancelActiveRequest, updateAssistant]);

  const reset = React.useCallback(() => {
    cancelActiveRequest();
    setMessages([]);
    setIsStreaming(false);
    setError(null);
    setPendingConsent(null);
    setSessionId(null);
  }, [cancelActiveRequest]);

  const forgetSession = React.useCallback((id?: string) => {
    setSessionId((current) =>
      id === undefined || current === id ? null : current,
    );
  }, []);

  const loadConversation = React.useCallback(
    (entries: StoredChatEntry[], nextSessionId: string | null) => {
      if (abortRef.current) return;
      setMessages(
        entries.map((entry) => ({
          id: makeId(),
          role: entry.role,
          content: entry.content,
          status: "done" as const,
        })),
      );
      setIsStreaming(false);
      setError(null);
      setPendingConsent(null);
      setSessionId(nextSessionId);
    },
    [],
  );

  return {
    messages,
    isStreaming,
    error,
    pendingConsent,
    sessionId,
    send,
    sendConsent,
    abort,
    reset,
    loadConversation,
    forgetSession,
  };
}
