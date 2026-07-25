export type ProviderId = "codex" | "claude" | "opencode";

export type BrokerModel = {
  id: string;
  display_name?: string;
  owned_by?: string;
  source_provider?: string;
  privacy_posture?: "local" | "remote" | "tee";
  privacy_reason?: string;
  supports_reasoning_effort?: boolean;
  reasoning_efforts?: string[];
};

export type ProviderStatus = {
  provider: ProviderId;
  display_name: string;
  executable?: string;
  state: "ready" | "missing_executable" | "authentication_required" | "error";
  version?: string;
  message: string;
  privacy_posture: "remote";
  native_tools: "disabled";
  models: BrokerModel[];
};

/**
 * Every adapter reports the same shape: a remote provider whose native tools are
 * off, with the state/message/models varying per probe outcome. The invariants
 * live here so the adapters only spell out what differs.
 */
export function providerStatus(
  provider: ProviderId,
  display_name: string,
  rest: Pick<ProviderStatus, "state" | "message"> &
    Partial<Pick<ProviderStatus, "executable" | "version" | "models">>,
): ProviderStatus {
  return {
    provider,
    display_name,
    privacy_posture: "remote",
    native_tools: "disabled",
    models: [],
    ...rest,
  };
}

export type ChatRequest = {
  command: "chat";
  request_id: string;
  provider: ProviderId;
  model: string;
  messages: Array<{ role: string; content: string }>;
  options?: { reasoning_effort?: string; provider_session_id?: string };
};

export type BrokerRequest =
  | { command: "status" }
  | { command: "models"; provider: ProviderId }
  | ChatRequest;

export type BrokerEvent =
  | { type: "status"; phase: string; message: string }
  | { type: "delta"; content?: string; reasoning?: string }
  | { type: "done"; finish_reason: string; provider_session_id?: string }
  | {
      type: "error";
      code: "missing_executable" | "authentication_required" | "provider_error";
      message: string;
      hint?: string;
    }
  | { type: "result"; data: unknown };

export function writeEvent(event: BrokerEvent): void {
  process.stdout.write(`${JSON.stringify(event)}\n`);
}

export function safeErrorMessage(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  const firstLine = raw.split(/\r?\n/, 1)[0]?.trim() || "Provider request failed.";
  return firstLine
    .replace(/(bearer|token|api[_ -]?key|authorization)\s*[:=]\s*\S+/gi, "$1=[redacted]")
    .slice(0, 400);
}
