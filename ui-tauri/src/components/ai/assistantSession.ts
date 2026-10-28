import * as React from "react";
import type { BookStamp } from "./bookStamp";
import type { EvidenceRequest, EvidenceRequestState } from "./evidenceRequest";

import type {
  AiChatMessage,
  AiPermissionMode,
  AiToolConsentDecision,
  AiToolConsentRequest,
  AiToolProfile,
} from "@/daemon/stream";

export type AssistantReturnPath =
  | "/overview"
  | "/transactions"
  | "/activity"
  | "/reports"
  | "/privacy-mirror"
  | "/chain-analysis"
  | "/exit-tax"
  | "/source-of-funds"
  | "/connections"
  | "/books"
  | "/journals"
  | "/swaps"
  | "/custody-gaps"
  | "/quarantine"
  | "/reconcile"
  | "/egress"
  | "/imports"
  | "/logs"
  | "/settings";

export type AssistantToolCapability =
  | "core"
  | "workspace"
  | "transactions"
  | "reports"
  | "wallets"
  | "loans"
  | "privacy"
  | "source_funds"
  | "merchant"
  | "transfers"
  | "review"
  | "operations";

/**
 * Counts a page shows that the assistant may see. The daemon owns the names
 * and their definitions (`SCREEN_CONTEXT_METRIC_DEFINITIONS`) and rejects
 * anything else; values are bare counts, never identities or amounts.
 */
export type AssistantScreenMetric =
  | "review_queue_review"
  | "review_queue_pending"
  | "review_queue_failed"
  | "incoming_count"
  | "outgoing_count";

export type AssistantScreenMetrics = Partial<Record<AssistantScreenMetric, number>>;

/**
 * The book state a page's metrics were counted on, echoed from the daemon's
 * snapshot. The daemon drops the metrics unless this still matches the active
 * book and its journal input version.
 */
export interface AssistantMetricsScope {
  workspaceId: string;
  profileId: string;
  inputVersion: number;
  /**
   * Id of the one wallet the page's wallet filter selects, or null for every
   * wallet. Never a descriptor, xpub or wallet configuration.
   */
  wallet: string | null;
  /**
   * How the page counted its review queue: reviewed pairs as one item (current
   * journals) or each leg separately. The model's metric definitions follow
   * it, and the daemon drops counts made under another journal state.
   */
  pairsGrouped: boolean;
}

export interface AssistantScreenContext {
  route: AssistantReturnPath;
  entityType?:
    | "transaction"
    | "wallet"
    | "report"
    | "source_funds_case"
    | "connection"
    | "custody_gap"
    | "profile";
  entityId?: string;
  filters?: Record<string, unknown>;
  capabilities: AssistantToolCapability[];
  metrics?: AssistantScreenMetrics;
  metricsScope?: AssistantMetricsScope;
}

/**
 * Marks a prompt as a page's own request to the assistant (a handoff: the
 * quarantine fix, Source of Funds, chain analysis, PSBT, datasets). It runs
 * with the screen context of the page that made it, captured when it is made,
 * so an "Ask about this" pin can never replace that page's tool pack; the pin
 * is cleared instead. Start one with `beginAssistantHandoff`.
 */
export interface AssistantHandoff {
  /** Defaults to the page on screen when the handoff is made. */
  context?: AssistantScreenContext;
  /**
   * The book the page's content belongs to, captured at the user's action
   * before any await (see `BookStamp`). The request names it, and it is not
   * sent while another book is open.
   */
  book: BookStamp;
}

/**
 * An item the user pinned with "Ask about this". Its typed context replaces
 * the route-derived one for the following turns until removed; `label` is
 * local display text for the composer chip and is never sent.
 */
export interface AssistantFocus {
  context: AssistantScreenContext;
  label: string;
}

export interface AssistantModelSelection {
  provider: string;
  model: string;
}

export type AssistantThinkingEffort =
  | "auto"
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "max"
  | "ultra";

export interface AssistantAttachment {
  /** Opaque daemon staging grant; expires with the daemon's staging TTL. */
  token: string;
  filename: string;
  kind?: string;
  sizeBytes?: number;
  /** A small data: URL thumbnail of a pasted image; never a path. */
  previewUrl?: string;
}

export interface AssistantSessionContextValue {
  evidenceRequests?: Record<string, EvidenceRequestState>;
  openEvidenceRequest?: (request: EvidenceRequest) => Promise<void>;
  continueEvidenceRequest?: (request: EvidenceRequest, unavailable?: boolean) => void;
  messages: AiChatMessage[];
  isStreaming: boolean;
  error: { code: string; message: string } | null;
  /**
   * Approvals the running answer waits on, each shown as a row in the chat
   * work log beside its tool call. Rarely more than one.
   */
  pendingConsents: AiToolConsentRequest[];
  queuedPrompts: string[];
  selection: AssistantModelSelection | null;
  thinkingEffort: AssistantThinkingEffort;
  /**
   * Fast serving mode for the next turns. Always false while the selected
   * model does not offer it, so it is never sent to a model that cannot
   * honour it.
   */
  fastMode: boolean;
  returnPath: AssistantReturnPath;
  /** Persisted session backing this conversation, if any. */
  sessionId: string | null;
  /** When true, turns are sent with persist=false (nothing stored). */
  incognito: boolean;
  /**
   * File attached to this conversation, for the assistant's file-analysis tool.
   * Holds a staging token and the display filename — never a path, and never
   * the file's contents.
   */
  attachment: AssistantAttachment | null;
  setSelection: (next: AssistantModelSelection | null) => void;
  setThinkingEffort: (next: AssistantThinkingEffort) => void;
  /**
   * Why the composer cannot send right now (this book's provider settings are
   * still loading, so the effort is not seeded yet); null when it can.
   */
  sendBlockedReason?: string | null;
  setFastMode: (next: boolean) => void;
  setIncognito: (next: boolean) => void;
  /**
   * `uploading` while a pasted file is handed over (shown from `attachment`),
   * `sent` once a message carried the chat's file.
   */
  attachmentState?: "uploading" | "ready" | "sent";
  /** Open the native picker and attach the chosen file to this conversation. */
  attachFile: () => Promise<void>;
  /** Attach a pasted file, or one dropped in the browser preview. */
  attachFiles?: (files: File[]) => Promise<void>;
  /** Attach the file just dropped on the desktop window. */
  attachDroppedFile?: () => Promise<void>;
  clearAttachment: () => void;
  /**
   * Send a prompt; `handoff` marks a page's own request (see AssistantHandoff).
   * A plain prompt belongs to the book open when this is called, the user's
   * submit. Without a chosen model a handoff waits in the composer with its
   * page and book; a plain prompt is not sent. Returns whether the prompt was
   * taken (sent, queued or parked); one refused at once returns false, and
   * the composer keeps it.
   */
  sendPrompt: (prompt: string, handoff?: AssistantHandoff) => boolean;
  /**
   * The open book's stamp, for content the user's action creates now; null
   * while its ids are not known, and the action is then not taken.
   */
  captureBook?: () => BookStamp | null;
  sendConsent: (callId: string, decision: AiToolConsentDecision) => Promise<void>;
  /**
   * This chat's permission mode, sent with every turn. A new chat starts in
   * the Settings default; a reopened chat in the mode it last used.
   */
  permissionMode?: AiPermissionMode;
  /**
   * Choose this chat's mode. While an answer streams it also switches that
   * answer, and the daemon releases waiting approvals the new mode covers.
   */
  setPermissionMode?: (mode: AiPermissionMode) => void;
  /**
   * The tools the next turn offers the model: this chat's pick for the
   * selected provider, else that provider's setting, else all tools.
   */
  toolProfile?: AiToolProfile;
  /** This chat's pick for the selected provider (from the next turn on). */
  setToolProfile?: (profile: AiToolProfile) => void;
  abort: () => void;
  reset: () => void;
  /** Load a persisted session into the conversation. */
  resumeSession: (sessionId: string) => Promise<void>;
  /**
   * Fork a fresh, unsaved conversation seeded with history up to and including
   * the given message. The original chat is left intact in History.
   */
  branchFromMessage: (messageId: string) => void;
  /**
   * Edit a user message in place. When `nextContent` is provided (inline
   * editor confirm), the conversation is rewound to just before that prompt
   * and regenerated from the edited text in a fresh, unsaved turn. When it is
   * omitted, the prompt text is dropped back into the composer for a manual
   * resend (legacy rollback path).
   */
  editUserMessage: (messageId: string, nextContent?: string) => void;
  /** Drop the session binding (all sessions, or only when it matches id). */
  forgetSession: (id?: string) => void;
  /** Item pinned by "Ask about this", if any. */
  focus?: AssistantFocus | null;
  /**
   * Pin an item as typed context, prefill the composer with `prompt` (which
   * must not contain the item's identity), and open the panel. Never sends.
   */
  askAbout?: (focus: AssistantFocus, prompt: string) => void;
  clearFocus?: () => void;
}

export const AssistantSessionContext =
  React.createContext<AssistantSessionContextValue | null>(null);

export function useAssistantSession(): AssistantSessionContextValue {
  const context = React.useContext(AssistantSessionContext);
  if (!context) {
    throw new Error(
      "useAssistantSession must be used within AssistantSessionProvider",
    );
  }
  return context;
}
