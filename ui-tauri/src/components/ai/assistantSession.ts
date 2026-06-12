import * as React from "react";

import type {
  AiChatMessage,
  AiToolConsentDecision,
  AiToolConsentRequest,
} from "@/daemon/stream";

export type AssistantReturnPath =
  | "/overview"
  | "/transactions"
  | "/reports"
  | "/source-of-funds"
  | "/connections"
  | "/books"
  | "/journals"
  | "/quarantine"
  | "/imports"
  | "/logs"
  | "/settings";

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
