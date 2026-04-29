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
  | "/connections"
  | "/profiles"
  | "/journals"
  | "/tax-events"
  | "/quarantine"
  | "/settings";

export interface AssistantModelSelection {
  provider: string;
  model: string;
}

export interface AssistantSessionContextValue {
  messages: AiChatMessage[];
  isStreaming: boolean;
  error: { code: string; message: string } | null;
  pendingConsent: AiToolConsentRequest | null;
  selection: AssistantModelSelection | null;
  toolsEnabled: boolean;
  returnPath: AssistantReturnPath;
  setSelection: (next: AssistantModelSelection | null) => void;
  setToolsEnabled: (enabled: boolean) => void;
  sendPrompt: (prompt: string) => void;
  sendConsent: (decision: AiToolConsentDecision) => Promise<void>;
  abort: () => void;
  reset: () => void;
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
