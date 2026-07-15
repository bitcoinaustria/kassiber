import * as React from "react";

import type {
  AiChatMessage,
  AiToolConsentDecision,
  AiToolConsentRequest,
} from "@/daemon/stream";

export type AssistantReturnPath =
  | "/overview"
  | "/transactions"
  | "/activity"
  | "/reports"
  | "/privacy-mirror"
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
  | "operations";

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
}

export interface AssistantModelSelection {
  provider: string;
  model: string;
}

export type AssistantThinkingEffort = "auto" | "low" | "medium" | "high";

export interface AssistantSessionContextValue {
  messages: AiChatMessage[];
  isStreaming: boolean;
  error: { code: string; message: string } | null;
  pendingConsent: AiToolConsentRequest | null;
  queuedPrompts: string[];
  selection: AssistantModelSelection | null;
  thinkingEffort: AssistantThinkingEffort;
  returnPath: AssistantReturnPath;
  /** Persisted session backing this conversation, if any. */
  sessionId: string | null;
  /** When true, turns are sent with persist=false (nothing stored). */
  incognito: boolean;
  setSelection: (next: AssistantModelSelection | null) => void;
  setThinkingEffort: (next: AssistantThinkingEffort) => void;
  setIncognito: (next: boolean) => void;
  sendPrompt: (prompt: string) => void;
  sendConsent: (decision: AiToolConsentDecision) => Promise<void>;
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
