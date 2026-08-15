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
  const { executable, ...remainder } = rest;
  return {
    provider,
    display_name,
    privacy_posture: "remote",
    native_tools: "disabled",
    models: [],
    ...remainder,
    // The resolved path identifies the user (`/Users/<name>/.local/bin/...`) and
    // this object is forwarded verbatim to the UI. The basename is all the
    // surface needs to say which binary answered.
    ...(executable === undefined
      ? {}
      : { executable: executable.split(/[/\\]/).pop() || executable }),
  };
}

export type ChatRequest = {
  command: "chat";
  request_id: string;
  provider: ProviderId;
  model: string;
  messages: Array<{ role: string; content: string }>;
  instructions?: string;
  tools?: BrokerToolDefinition[];
  options?: { reasoning_effort?: string; provider_session_id?: string };
};

export type BrokerToolDefinition = {
  name: string;
  description: string;
  parameters: Record<string, unknown>;
  read_only?: boolean;
  destructive?: boolean;
};

export type BrokerToolResult = {
  call_id: string;
  output: string;
};

export type BrokerRequest =
  | { command: "status" }
  | { command: "models"; provider: ProviderId }
  | ChatRequest;

export type BrokerEvent =
  | { type: "status"; phase: string; message: string }
  | { type: "delta"; content?: string; reasoning?: string }
  | { type: "tool_call"; call_id: string; name: string; arguments: Record<string, unknown> }
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

/**
 * Accept a provider session cursor only if it looks like one.
 *
 * These cursors are echoed back to us from a prior turn and then handed to a
 * CLI. A value starting with `-` would be parsed as a new top-level flag rather
 * than an id, which turns a resumed chat into arbitrary CLI configuration
 * (`--dangerously-skip-permissions`, `--mcp-config=…`, `--debug-file=…`). The
 * three providers all issue id-shaped cursors, so anything outside that shape
 * is dropped and the turn simply starts a fresh provider session.
 */
export function safeSessionCursor(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  if (!trimmed || trimmed.length > 200) return undefined;
  return /^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(trimmed) ? trimmed : undefined;
}

/**
 * Sanitize a provider failure for the event stream.
 *
 * Provider CLIs put credentials, URLs and absolute home paths in their error
 * text, so every pattern below has to be handled, not just `key: value`:
 * a bare `Bearer <token>` after a redacted label, JSON `"apiKey": "..."`,
 * `https://user:pass@host`, `sk-`/`ghp_`-style tokens, and `/Users/<name>` or
 * `/home/<name>` prefixes. Each rule replaces the *secret*, not just the label
 * — an earlier version rewrote `authorization: Bearer sk-x` to
 * `authorization=[redacted] sk-x`, leaving the token in place.
 */
export function safeErrorMessage(error: unknown): string {
  const raw = error instanceof Error ? error.message : String(error);
  const firstLine = raw.split(/\r?\n/, 1)[0]?.trim() || "Provider request failed.";
  return (
    firstLine
      // URL userinfo, before anything else rewrites the URL.
      .replace(/:\/\/[^/@\s]+@/g, "://[redacted]@")
      // JSON or quoted assignment: "apiKey": "secret".
      .replace(
        /(["']?(?:bearer|token|api[_ -]?key|authorization|secret|password|passphrase)["']?\s*[:=]\s*)(["']?)[^"',;\s]+\2/gi,
        "$1[redacted]",
      )
      // A bare scheme + credential, whether or not a label preceded it.
      .replace(/\b(bearer|basic)\s+\S+/gi, "$1 [redacted]")
      // Vendor-shaped tokens standing alone.
      .replace(/\b(sk|pk|rk)-[A-Za-z0-9_-]{8,}/g, "[redacted]")
      .replace(/\b(gh[pousr]|xox[abposr])_[A-Za-z0-9_-]{8,}/g, "[redacted]")
      // Home directories identify the user; keep the leaf for orientation.
      .replace(/\/(?:Users|home)\/[^/\s]+/g, "~")
      .slice(0, 400)
  );
}
