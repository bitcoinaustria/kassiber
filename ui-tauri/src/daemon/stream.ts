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
import { useQueryClient } from "@tanstack/react-query";

import type {
  DaemonEnvelope,
  DaemonStreamRecord,
} from "./transport";
import { invalidateDaemonQueriesForMutation } from "./client";
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
  /**
   * Provider notices for this answer, by status phase (for example
   * `fast_mode_unavailable`). Rendered as translated copy, never the label.
   */
  notices?: AiChatNotice[];
  toolCalls?: AiChatToolCall[];
  status: "pending" | "streaming" | "done" | "error" | "cancelled";
  errorCode?: string;
  errorMessage?: string;
  finishReason?: string | null;
  provider?: string;
  model?: string;
  provenance?: AiAnswerProvenance;
  /** The files a user message first sent to the chat; display only. */
  attachments?: AiChatMessageAttachment[];
}

/** What a sent message shows about its file: never a path or a token. */
export interface AiChatMessageAttachment {
  filename: string;
  kind?: string;
  sizeBytes?: number;
  previewUrl?: string;
}

export interface AiAnswerProvenance {
  generated_at?: string;
  provider?: string;
  model?: string;
  /** Recognised provider notices for this answer, e.g. `fast_mode_unavailable`. */
  notices?: string[];
  tools_used?: string[];
  tools_attempted?: string[];
  tool_denials?: Array<{ tool: string; reason: string }>;
  active_transactions?: number | null;
  quarantines?: number | null;
  missing_price_transactions?: number | null;
  journals_processed_at?: string | null;
  auto_journal_processed?: boolean;
  auto_sync_attempted?: boolean;
  auto_sync_ok?: boolean | null;
  privacy_receipt?: {
    provider_kind?: string | null;
    remote_provider?: boolean;
    screen_route?: string | null;
    advertised_tool_count?: number | null;
    tools_attempted?: number;
    tools_executed?: number;
    tools_denied?: number;
    egress_records?: number;
    egress_endpoints?: number;
    egress_bytes_out?: number;
    egress_subsystems?: string[];
    egress_gap?: boolean;
    history_intent?: boolean | "auto" | null;
    hostnames_disclosed_to_model?: boolean;
    cross_book_data_disclosed?: boolean;
  };
}

export type AiToolCallStatus =
  | "pending"
  | "awaiting_consent"
  | "running"
  | "done"
  | "denied"
  | "error";

export type AiToolConsentDecision = "allow_once" | "allow_session" | "deny";

/**
 * How much the assistant may do without asking, per chat. `approval` asks
 * before every change; `auto` lets the daemon approve changes that can be
 * undone and stay on this machine; `full_access` never asks, network actions
 * included. The daemon decides which calls a mode covers; the desktop only
 * names the mode.
 */
export const AI_PERMISSION_MODES = ["approval", "auto", "full_access"] as const;
export type AiPermissionMode = (typeof AI_PERMISSION_MODES)[number];

export function isAiPermissionMode(value: unknown): value is AiPermissionMode {
  return (AI_PERMISSION_MODES as readonly unknown[]).includes(value);
}

/**
 * Which tools a turn offers the model (`tool_profile`): `full` every tool,
 * `scoped` the packs routed to the question (the model can load more areas),
 * `core` the common core only. The user picks; nothing is inferred from the
 * model.
 */
export const AI_TOOL_PROFILES = ["full", "scoped", "core"] as const;
export type AiToolProfile = (typeof AI_TOOL_PROFILES)[number];

export function isAiToolProfile(value: unknown): value is AiToolProfile {
  return (AI_TOOL_PROFILES as readonly unknown[]).includes(value);
}

/** Why a tool call ran (or waits): the daemon's `approval` on `ai.chat.tool_call`. */
export type AiToolApproval =
  | "required"
  | "auto"
  | "full_access"
  | "session"
  | "not_needed";

const AI_TOOL_APPROVALS: readonly AiToolApproval[] = [
  "required",
  "auto",
  "full_access",
  "session",
  "not_needed",
];

function aiToolApproval(value: unknown): AiToolApproval | undefined {
  return (AI_TOOL_APPROVALS as readonly unknown[]).includes(value)
    ? (value as AiToolApproval)
    : undefined;
}

// Mirrors the daemon's once-only consent tools. The daemon's
// `can_allow_session` decides; this list only narrows it (and stands in for
// it when an older daemon omits the field).
const AI_TOOL_ONCE_ONLY_CONSENT = new Set([
  "ui.journals.quarantine.resolve",
  "ui.transfers.components.apply",
  "ui.review.apply",
  "ui.wallets.import_attachment.apply",
  "ui.imports.rollback",
]);

export function aiToolAllowsSessionConsent(name: string): boolean {
  return !AI_TOOL_ONCE_ONLY_CONSENT.has(name);
}

export interface AiChatToolCall {
  callId: string;
  name: string;
  arguments: Record<string, unknown>;
  kindClass: "read_only" | "mutating" | "unknown";
  needsConsent: boolean;
  status: AiToolCallStatus;
  /** Why it ran without a prompt, or that it needs one (daemon `approval`). */
  approval?: AiToolApproval;
  result?: unknown;
  reason?: string;
  /** Server-revalidated effects, separate from the model's tool arguments. */
  reviewPreview?: AiReviewPreview;
  /** Reasoning segment (provider completion round) this call belongs to. */
  segmentId?: string;
}

export interface AiChatRequest {
  expectedScope?: { workspace_id: string; profile_id: string };
  /** This chat's permission mode; the daemon treats absent as `approval`. */
  permissionMode?: AiPermissionMode;
  provider?: string;
  model: string;
  messages: { role: AiChatMessage["role"] | "tool"; content: string }[];
  options?: Record<string, unknown>;
  toolsEnabled?: boolean;
  /** The tool set the user chose; the daemon defaults absent to `scoped`. */
  toolProfile?: AiToolProfile;
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
  /**
   * Notices of the seeded answers a fork carries over, by `messages` index, so
   * the new session stores them with its seed. Storage only: the daemon keeps
   * them out of the model prompt.
   */
  seedNotices?: { index: number; notices: AiChatNotice[] }[];
  /**
   * The files attached to this chat, in order, as opaque staging grants. A
   * token and never a path: the renderer does not learn where a file is, and
   * the model cannot point a tool elsewhere. File contents are not sent here;
   * the daemon reads the files locally. `new` marks the files this message
   * first sends.
   */
  attachments?: { token: string; label?: string | null; new?: boolean }[];
  /** Typed, ephemeral UI state. Never includes descriptors, file content, or secrets. */
  screenContext?: {
    route?: string;
    entityType?:
      | "transaction"
      | "wallet"
      | "report"
      | "source_funds_case"
      | "custody_gap"
      | "connection"
      | "profile";
    entityId?: string;
    filters?: Record<string, unknown>;
    capabilities?: string[];
    /** Bare counts the page shows, keyed by the daemon's metric allowlist. */
    metrics?: Record<string, number>;
    /** Book state the metrics were counted on; required with `metrics`. */
    metricsScope?: {
      workspaceId: string;
      profileId: string;
      inputVersion: number;
      wallet: string | null;
      pairsGrouped: boolean;
    };
  };
}

/** One stored exchange row from ui.chat.sessions.get. */
export interface StoredChatEntry {
  role: "user" | "assistant";
  content: string;
  /** Notices kept with an assistant answer, so they survive a reload. */
  notices?: AiChatNotice[];
}

/** A message row as `ui.chat.sessions.get` returns it. */
export interface StoredChatMessageShape {
  role?: string;
  content?: string;
  provenance?: { notices?: unknown } | null;
}

/**
 * The conversation to show for a stored session: user and assistant rows with
 * text, each assistant answer keeping the notices its provenance recorded.
 */
export function storedChatEntries(
  messages: readonly StoredChatMessageShape[],
): StoredChatEntry[] {
  const entries: StoredChatEntry[] = [];
  for (const message of messages) {
    const { role, content } = message;
    if (role !== "user" && role !== "assistant") continue;
    if (typeof content !== "string" || content.length === 0) continue;
    const notices =
      role === "assistant" ? aiChatNotices(message.provenance?.notices) : [];
    entries.push(notices.length ? { role, content, notices } : { role, content });
  }
  return entries;
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
    tool_profile: request.toolProfile,
    tool_loop_max_iterations: request.toolLoopMaxIterations,
    system_prompt_kind: request.systemPromptKind,
    system_prompt: request.systemPrompt,
    session_id: request.sessionId ?? undefined,
    expected_scope: request.expectedScope,
    permission_mode: request.permissionMode,
    persist: request.persist,
    seed_history: request.seedHistory ? true : undefined,
    seed_notices:
      request.seedHistory && request.seedNotices?.length ? request.seedNotices : undefined,
    attachments: request.attachments?.length
      ? request.attachments.map((attachment) => ({
          token: attachment.token,
          label: attachment.label ?? undefined,
          new: attachment.new ? true : undefined,
        }))
      : undefined,
    screen_context: request.screenContext
      ? {
          route: request.screenContext.route,
          entity_type: request.screenContext.entityType,
          entity_id: request.screenContext.entityId,
          filters: request.screenContext.filters,
          capabilities: request.screenContext.capabilities,
          metrics: request.screenContext.metrics,
          metrics_scope: request.screenContext.metricsScope
            ? {
                workspace_id: request.screenContext.metricsScope.workspaceId,
                profile_id: request.screenContext.metricsScope.profileId,
                input_version: request.screenContext.metricsScope.inputVersion,
                wallet: request.screenContext.metricsScope.wallet,
                pairs_grouped: request.screenContext.metricsScope.pairsGrouped,
              }
            : undefined,
        }
      : undefined,
  };
}

export type AiReviewPreview =
  | { status: "ready"; artifact: unknown }
  | { status: "applied"; receipt: unknown }
  | { status: "unavailable"; code: string };

export interface AiToolConsentRequest {
  targetRequestId: string;
  callId: string;
  name: string;
  summary: string;
  argumentsPreview: Record<string, unknown>;
  reviewPreview?: AiReviewPreview;
  /** The daemon can undo it (an older daemon that does not say: no). */
  reversible: boolean;
  /** It contacts the network (an older daemon that does not say: no). */
  egresses: boolean;
  /** "Allow for this chat" may be offered for this call. */
  canAllowSession: boolean;
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
  approval?: AiToolApproval;
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
  review_preview?: AiReviewPreview;
  reversible?: boolean;
  egresses?: boolean;
  can_allow_session?: boolean;
}

export function buildAiToolConsentRequest(
  targetRequestId: string,
  data: AiChatToolConsentRequiredShape,
): AiToolConsentRequest {
  return {
    targetRequestId,
    callId: data.call_id,
    name: data.name,
    summary: data.summary ?? data.name,
    argumentsPreview: data.arguments_preview ?? {},
    reviewPreview: data.review_preview,
    // An older daemon omits these: then nothing claims the call can be
    // undone, and session consent follows the once-only mirror above.
    reversible: data.reversible === true,
    egresses: data.egresses === true,
    canAllowSession:
      data.can_allow_session !== false && aiToolAllowsSessionConsent(data.name),
  };
}

interface AiToolConsentResponseShape {
  recorded?: boolean;
  reason?: string;
}

interface AiPermissionModeResponseShape {
  permission_mode?: string;
  released_call_ids?: unknown;
}

/** What a mid-turn permission-mode change did to the running answer. */
export interface AiPermissionModeChange {
  /** The running turn now uses the new mode. */
  applied: boolean;
  /** Pending approvals the new mode covers, which the daemon released. */
  releasedCallIds: string[];
  /** Set when the daemon refused or could not be reached. */
  error?: { code: string; message: string };
}

export function buildPermissionModeArgs(
  targetRequestId: string,
  mode: AiPermissionMode,
): Record<string, unknown> {
  return { target_request_id: targetRequestId, permission_mode: mode };
}

export type AiChatStreamRecordData =
  | AiChatStatusShape
  | AiChatDeltaShape
  | AiChatToolCallShape
  | AiChatToolResultShape
  | AiChatToolConsentRequiredShape;

interface ObservedAiToolCall {
  name: string;
  kindClass: AiChatToolCall["kindClass"];
}

/**
 * Track the streamed tool lifecycle and return a daemon mutation kind exactly
 * once, only when the matching mutating call completed successfully.
 */
export function completedAiMutationKind(
  observedCalls: Map<string, ObservedAiToolCall>,
  record: DaemonStreamRecord<AiChatStreamRecordData>,
): string | null {
  if (record.kind === "ai.chat.tool_call") {
    const data = record.data as AiChatToolCallShape | undefined;
    if (data?.call_id && data.name) {
      observedCalls.set(data.call_id, {
        name: data.name,
        kindClass: data.kind_class ?? "unknown",
      });
    }
    return null;
  }
  if (record.kind !== "ai.chat.tool_result") return null;
  const data = record.data as AiChatToolResultShape | undefined;
  if (!data?.call_id) return null;
  const observed = observedCalls.get(data.call_id);
  observedCalls.delete(data.call_id);
  return data.ok === true && observed?.kindClass === "mutating"
    ? observed.name
    : null;
}

export interface UseAiChatStreamResult {
  messages: AiChatMessage[];
  isStreaming: boolean;
  error: { code: string; message: string } | null;
  /** Approvals the running turn waits on, in arrival order (rarely more than one). */
  pendingConsents: AiToolConsentRequest[];
  /** Persisted session backing this conversation, if any. */
  sessionId: string | null;
  /**
   * Resolves `{ refused: true }` when the daemon refused the turn before it
   * started because another book is open (`stale_context` against the
   * request's `expectedScope`): nothing ran, and the turn is not kept in the
   * transcript, so the caller decides what happens to its text.
   */
  send: (
    request: AiChatRequest,
    userMessageContent: string,
    userAttachments?: AiChatMessageAttachment[],
  ) => Promise<{ refused: boolean }>;
  sendConsent: (callId: string, decision: AiToolConsentDecision) => Promise<void>;
  /**
   * Switch the running turn to `mode` (`ai.chat.permission_mode`). Nothing
   * is sent while no turn runs; the next turn names its mode itself.
   */
  changePermissionMode: (mode: AiPermissionMode) => Promise<AiPermissionModeChange>;
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

/** One provider completion round: its reasoning trace plus its tool calls. */
export interface AiChatRound {
  id: string;
  thinking: string;
  toolCalls: AiChatToolCall[];
}

/**
 * Group an assistant turn into rounds so reasoning and the tool calls it led
 * to render together (think → tools → think again). Tool calls without a
 * segment (restored history, calls before any reasoning) lead the first round.
 */
export function chatMessageRounds(message: AiChatMessage): AiChatRound[] {
  const toolCalls = message.toolCalls ?? [];
  const segments =
    message.thinkingSegments && message.thinkingSegments.length > 0
      ? message.thinkingSegments
      : visibleThinkingSegments(message);
  if (segments.length === 0) {
    return toolCalls.length > 0
      ? [{ id: `${message.id}-round`, thinking: "", toolCalls }]
      : [];
  }
  const knownIds = new Set(segments.map((segment) => segment.id));
  const rounds = segments.map((segment) => ({
    id: segment.id,
    thinking: segment.content,
    toolCalls: toolCalls.filter((call) => call.segmentId === segment.id),
  }));
  const untagged = toolCalls.filter(
    (call) => !call.segmentId || !knownIds.has(call.segmentId),
  );
  rounds[0]!.toolCalls = [...untagged, ...rounds[0]!.toolCalls];
  return rounds.filter(
    (round) => round.thinking.length > 0 || round.toolCalls.length > 0,
  );
}

/**
 * Open a new reasoning segment for the next provider completion round.
 * Skips when the current segment is still empty so repeated status
 * records do not create blank panes — unless that segment already owns tool
 * calls, which would otherwise fold two rounds into one pane.
 */
export function beginThinkingSegment(current: AiChatMessage): AiChatMessage {
  const segments = current.thinkingSegments ?? [];
  const last = segments[segments.length - 1];
  const lastHasTools = last
    ? (current.toolCalls ?? []).some((call) => call.segmentId === last.id)
    : false;
  if (last && last.content.length === 0 && !lastHasTools) {
    return current;
  }
  return {
    ...current,
    thinkingSegments: [...segments, { id: makeId(), content: "" }],
  };
}

/** Reasoning segment a freshly streamed tool call belongs to, if any. */
function activeSegmentId(current: AiChatMessage): string | undefined {
  const segments = current.thinkingSegments ?? [];
  return segments[segments.length - 1]?.id;
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
  request: Pick<AiToolConsentRequest, "targetRequestId" | "callId">,
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

/**
 * Status phases that describe the answer rather than progress: a CLI agent
 * declined fast mode, or an HTTP endpoint refused the reasoning effort and
 * answered without it. Mirrors the daemon's `PROVIDER_STATUS_NOTICES`.
 */
export const AI_CHAT_NOTICE_PHASES = [
  "fast_mode_unavailable",
  "reasoning_effort_unavailable",
] as const;
export type AiChatNotice = (typeof AI_CHAT_NOTICE_PHASES)[number];

function isAiChatNotice(phase: unknown): phase is AiChatNotice {
  return (AI_CHAT_NOTICE_PHASES as readonly unknown[]).includes(phase);
}

/** The recognised notices in `value`, once each; anything else is dropped. */
export function aiChatNotices(value: unknown): AiChatNotice[] {
  if (!Array.isArray(value)) return [];
  return AI_CHAT_NOTICE_PHASES.filter((phase) => value.includes(phase));
}

function mergeAiChatNotices(
  current: AiChatNotice[] | undefined,
  more: AiChatNotice[],
): AiChatNotice[] | undefined {
  const merged = [...(current ?? [])];
  for (const notice of more) if (!merged.includes(notice)) merged.push(notice);
  return merged.length ? merged : current;
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
    if (isAiChatNotice(phase)) {
      // A notice about the answer, not a loading hint: keep it on the message
      // and leave the progress label alone.
      const notices = current.notices ?? [];
      return notices.includes(phase)
        ? current
        : { ...current, notices: [...notices, phase] };
    }
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
    const approval = aiToolApproval(data.approval);
    const nextToolCall: AiChatToolCall = {
      callId: data.call_id,
      name: data.name,
      arguments: data.arguments ?? {},
      kindClass: data.kind_class ?? "unknown",
      needsConsent: Boolean(data.needs_consent),
      status: data.needs_consent ? "pending" : "running",
      // An older daemon sends no `approval`; keep what an earlier record said.
      ...(approval ? { approval } : {}),
      segmentId: activeSegmentId(current),
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
              segmentId: activeSegmentId(current),
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
      reviewPreview: data.review_preview,
      kindClass: "mutating",
      needsConsent: true,
      status: "awaiting_consent",
      approval: "required",
      segmentId: activeSegmentId(current),
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
  const queryClient = useQueryClient();
  const [messages, setMessages] = React.useState<AiChatMessage[]>([]);
  const [isStreaming, setIsStreaming] = React.useState(false);
  const [error, setError] = React.useState<UseAiChatStreamResult["error"]>(
    null,
  );
  const [pendingConsents, setPendingConsents] = React.useState<
    AiToolConsentRequest[]
  >([]);
  const pendingConsentsRef = React.useRef(pendingConsents);
  pendingConsentsRef.current = pendingConsents;
  const [sessionId, setSessionId] = React.useState<string | null>(null);
  const abortRef = React.useRef<AbortController | null>(null);
  const parserRef = React.useRef<ThinkParser | null>(null);
  const assistantIdRef = React.useRef<string | null>(null);
  const requestIdRef = React.useRef<string | null>(null);
  const observedToolCallsRef = React.useRef(new Map<string, ObservedAiToolCall>());
  const recordQueueRef = React.useRef<
    DaemonStreamRecord<AiChatStreamRecordData>[]
  >([]);
  const flushTimerRef = React.useRef<number | null>(null);
  const mountedRef = React.useRef(true);


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
      const completedMutation = completedAiMutationKind(
        observedToolCallsRef.current,
        record,
      );
      if (completedMutation) {
        invalidateDaemonQueriesForMutation(
          queryClient,
          useUiStore.getState().dataMode,
          completedMutation,
        );
      }
      if (record.kind === "ai.chat.tool_consent_required") {
        const data = record.data as AiChatToolConsentRequiredShape | undefined;
        const targetRequestId =
          typeof record.request_id === "string"
            ? record.request_id
            : requestIdRef.current;
        if (data?.call_id && data.name && targetRequestId) {
          const request = buildAiToolConsentRequest(targetRequestId, data);
          setPendingConsents((current) => [
            ...current.filter((item) => item.callId !== request.callId),
            request,
          ]);
        }
      }
      // The call left the approval: it has a result, or the daemon released
      // it to run (approved, or covered by a permission mode switched to
      // while it waited).
      const settledCallId =
        record.kind === "ai.chat.tool_result"
          ? (record.data as AiChatToolResultShape | undefined)?.call_id
          : record.kind === "ai.chat.tool_call" &&
              !(record.data as AiChatToolCallShape | undefined)?.needs_consent
            ? (record.data as AiChatToolCallShape | undefined)?.call_id
            : undefined;
      if (settledCallId) {
        setPendingConsents((current) =>
          current.some((item) => item.callId === settledCallId)
            ? current.filter((item) => item.callId !== settledCallId)
            : current,
        );
      }
      recordQueueRef.current.push(record);
      scheduleRecordFlush();
    },
    [queryClient, scheduleRecordFlush],
  );

  const cancelActiveRequest = React.useCallback(() => {
    const requestId = requestIdRef.current;
    abortRef.current?.abort();
    recordQueueRef.current = [];
    observedToolCallsRef.current.clear();
    if (flushTimerRef.current !== null) {
      window.clearTimeout(flushTimerRef.current);
      flushTimerRef.current = null;
    }
    if (requestId) {
      void getTransport()
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
    async (
      request: AiChatRequest,
      userMessageContent: string,
      userAttachments?: AiChatMessageAttachment[],
    ): Promise<{ refused: boolean }> => {
      if (isStreaming) return { refused: false };
      setError(null);
      setIsStreaming(true);
      setPendingConsents([]);
      const userId = makeId();
      const assistantId = makeId();
      assistantIdRef.current = assistantId;
      parserRef.current = new ThinkParser();
      observedToolCallsRef.current.clear();
      setMessages((prev) => [
        ...prev,
        {
          id: userId,
          role: "user",
          content: userMessageContent,
          status: "done",
          ...(userAttachments?.length ? { attachments: userAttachments } : {}),
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
      // False once the conversation is reset (New chat, or a book switch)
      // while this turn runs: what it returns belongs to the conversation it
      // left, never to the fresh one.
      const ownsConversation = () => requestIdRef.current === requestId;
      // Whether the turn started at all: a refusal before any record means
      // nothing ran.
      let started = false;
      try {
        const transport = getTransport();
        const envelope = (await transport.stream<
          AiChatTerminalShape,
          AiChatStreamRecordData
        >(
          {
            kind: "ai.chat",
            request_id: requestId,
            args: buildAiChatStreamArgs(request),
          },
          {
            onRecord: (record) => {
              started = true;
              if (ownsConversation()) onRecord(record);
            },
            signal: controller.signal,
          },
        )) as DaemonEnvelope<AiChatTerminalShape>;
        if (!mountedRef.current) return { refused: false };
        const failed = envelope.kind === "error" || Boolean(envelope.error);
        const code = envelope.error?.code ?? "unknown_error";
        // Asked in a book that is no longer open: the daemon refused it
        // before any model or tool saw it.
        const refused =
          failed && code === "stale_context" && !started && Boolean(request.expectedScope);
        if (!ownsConversation()) return { refused };
        flushQueuedRecords();

        if (failed) {
          const message = envelope.error?.message ?? "AI chat failed";
          if (refused) {
            // Drop the turn it never had.
            setMessages((prev) =>
              prev.filter(
                (message) => message.id !== userId && message.id !== assistantId,
              ),
            );
            return { refused: true };
          }
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
          return { refused: false };
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
          // The terminal result is the answer's record of its notices, the
          // same list a reload restores from history.
          notices: mergeAiChatNotices(
            current.notices,
            aiChatNotices(envelope.data?.provenance?.notices),
          ),
        }));
      } catch (caught) {
        if (!mountedRef.current || !ownsConversation()) return { refused: false };
        if (controller.signal.aborted) {
          updateAssistant((current) => ({
            ...current,
            status: "cancelled",
          }));
          return { refused: false };
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
        // A reset already cleared these, and a later turn may own them now.
        if (ownsConversation()) {
          if (flushTimerRef.current !== null) {
            window.clearTimeout(flushTimerRef.current);
            flushTimerRef.current = null;
          }
          recordQueueRef.current = [];
          observedToolCallsRef.current.clear();
          if (mountedRef.current) {
            setIsStreaming(false);
            setPendingConsents([]);
          }
          abortRef.current = null;
          parserRef.current = null;
          assistantIdRef.current = null;
          requestIdRef.current = null;
        }
      }
      return { refused: false };
    },
    [flushQueuedRecords, isStreaming, onRecord, updateAssistant],
  );

  const sendConsent = React.useCallback(
    async (callId: string, decision: AiToolConsentDecision) => {
      const request = pendingConsentsRef.current.find(
        (item) => item.callId === callId,
      );
      if (!request) return;
      // Answered once: the row goes at once, so a second click cannot send
      // another decision for the same call.
      pendingConsentsRef.current = pendingConsentsRef.current.filter(
        (item) => item.callId !== callId,
      );
      setPendingConsents((current) =>
        current.filter((item) => item.callId !== callId),
      );
      try {
        const envelope =
          await getTransport().invoke<AiToolConsentResponseShape>({
            kind: "ai.tool_call.consent",
            request_id: makeDaemonRequestId(),
            args: buildToolConsentArgs(request, decision),
          });
        // Answered for a turn a reset has since left: nothing to show here.
        if (!mountedRef.current || requestIdRef.current !== request.targetRequestId) return;
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
        // Answered for a turn a reset has since left: nothing to show here.
        if (!mountedRef.current || requestIdRef.current !== request.targetRequestId) return;
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
    [updateAssistant],
  );

  const changePermissionMode = React.useCallback(
    async (mode: AiPermissionMode): Promise<AiPermissionModeChange> => {
      const targetRequestId = requestIdRef.current;
      if (!targetRequestId) return { applied: false, releasedCallIds: [] };
      try {
        const envelope =
          await getTransport().invoke<AiPermissionModeResponseShape>({
            kind: "ai.chat.permission_mode",
            request_id: makeDaemonRequestId(),
            args: buildPermissionModeArgs(targetRequestId, mode),
          });
        // The turn ended (or a reset left it) meanwhile: the next turn names
        // its mode itself.
        if (!mountedRef.current || requestIdRef.current !== targetRequestId) {
          return { applied: false, releasedCallIds: [] };
        }
        if (envelope.kind === "error" || envelope.error) {
          return {
            applied: false,
            releasedCallIds: [],
            error: {
              code: envelope.error?.code ?? "permission_mode_failed",
              message:
                envelope.error?.message ?? "Could not change the permission mode",
            },
          };
        }
        const released = Array.isArray(envelope.data?.released_call_ids)
          ? envelope.data.released_call_ids.filter(
              (id): id is string => typeof id === "string",
            )
          : [];
        if (released.length > 0) {
          // The daemon runs these now; their `ai.chat.tool_call` records
          // follow and turn the cards to running.
          pendingConsentsRef.current = pendingConsentsRef.current.filter(
            (item) => !released.includes(item.callId),
          );
          setPendingConsents((current) =>
            current.filter((item) => !released.includes(item.callId)),
          );
        }
        return { applied: true, releasedCallIds: released };
      } catch (caught) {
        if (!mountedRef.current || requestIdRef.current !== targetRequestId) {
          return { applied: false, releasedCallIds: [] };
        }
        return {
          applied: false,
          releasedCallIds: [],
          error: {
            code: "permission_mode_failed",
            message: caught instanceof Error ? caught.message : String(caught),
          },
        };
      }
    },
    [],
  );

  const abort = React.useCallback(() => {
    cancelActiveRequest();
    setPendingConsents([]);
    updateAssistant((current) => ({
      ...current,
      status: "cancelled",
    }));
  }, [cancelActiveRequest, updateAssistant]);

  const reset = React.useCallback(() => {
    cancelActiveRequest();
    // Detach the cancelled turn, so its late result cannot reach the fresh
    // conversation (rebind the old session, show its error, or end a turn
    // started after it).
    abortRef.current = null;
    parserRef.current = null;
    assistantIdRef.current = null;
    requestIdRef.current = null;
    setMessages([]);
    setIsStreaming(false);
    setError(null);
    setPendingConsents([]);
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
          ...(entry.notices?.length ? { notices: entry.notices } : {}),
        })),
      );
      setIsStreaming(false);
      setError(null);
      setPendingConsents([]);
      setSessionId(nextSessionId);
    },
    [],
  );

  return {
    messages,
    isStreaming,
    error,
    pendingConsents,
    sessionId,
    send,
    sendConsent,
    changePermissionMode,
    abort,
    reset,
    loadConversation,
    forgetSession,
  };
}
