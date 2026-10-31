export type ProviderId = "codex" | "claude" | "opencode" | "copilot";

export type BrokerModel = {
  id: string;
  display_name?: string;
  owned_by?: string;
  source_provider?: string;
  privacy_posture?: "local" | "remote" | "tee";
  privacy_reason?: string;
  supports_reasoning_effort?: boolean;
  reasoning_efforts?: string[];
  /** The model offers a faster, higher-cost serving mode (Codex "Fast" tier, Claude Opus fast mode). */
  supports_fast_mode?: boolean;
  /** Provider wording for that mode, e.g. "2x speed, increased usage". */
  fast_mode_description?: string;
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

/**
 * An image the user attached, inline as base64. Never a path or a URL: the
 * daemon reads the file it holds a grant for and passes the bytes, so a
 * provider cannot be pointed at anything else.
 */
export type BrokerImage = { media_type: string; data: string };

export type ChatMessage = { role: string; content: string; images?: BrokerImage[] };

export const BROKER_IMAGE_TYPES: ReadonlySet<string> = new Set([
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
]);
// The daemon's own per-turn limits are lower; these only bound a bad request.
export const MAX_BROKER_IMAGES = 16;
export const MAX_BROKER_IMAGE_CHARS = 8 * 1024 * 1024;
const BASE64 = /^[A-Za-z0-9+/]+={0,2}$/;

/** The images of these messages, checked; a malformed one fails the turn. */
export function messageImages(messages: ChatMessage[]): BrokerImage[] {
  const images = messages.flatMap((message) => message.images ?? []);
  if (images.length > MAX_BROKER_IMAGES) throw new Error("Too many attached images.");
  for (const image of images) {
    if (
      typeof image?.media_type !== "string" ||
      !BROKER_IMAGE_TYPES.has(image.media_type) ||
      typeof image.data !== "string" ||
      image.data.length === 0 ||
      image.data.length > MAX_BROKER_IMAGE_CHARS ||
      !BASE64.test(image.data)
    ) {
      throw new Error("Invalid attached image.");
    }
  }
  return images;
}

export type ChatRequest = {
  command: "chat";
  request_id: string;
  provider: ProviderId;
  model: string;
  messages: ChatMessage[];
  instructions?: string;
  tools?: BrokerToolDefinition[];
  options?: {
    reasoning_effort?: string;
    provider_session_id?: string;
    sensitive_context?: boolean;
    /** An Incognito turn: ask the provider not to save its session. */
    stateless_session?: boolean;
    fast_mode?: boolean;
  };
};

/**
 * Whether the turn asked for the model's fast serving mode. Adapters apply it
 * only for models that advertise `supports_fast_mode`; anything that is not a
 * boolean is rejected rather than guessed.
 */
export function requestedFastMode(request: ChatRequest): boolean {
  const value = request.options?.fast_mode;
  if (value !== undefined && typeof value !== "boolean") {
    throw new Error("Invalid fast mode option.");
  }
  return value === true;
}

/** A selected-data grant is a single stateless exchange, never an agent session. */
export function sensitiveContext(request: ChatRequest): boolean {
  const value = request.options?.sensitive_context;
  if (value !== undefined && typeof value !== "boolean") {
    throw new Error("Invalid sensitive context mode.");
  }
  if (value === true && (request.tools?.length || request.options?.provider_session_id)) {
    throw new Error("Sensitive context cannot use tools or resume a provider session.");
  }
  return value === true;
}

/**
 * An Incognito turn: the provider is asked not to save its session (Codex
 * ephemeral thread, Claude `--no-session-persistence`, OpenCode in-memory
 * database), and no session cursor goes in or comes back, so the next turn
 * rebuilds from the visible transcript. Unlike selected-data turns it keeps
 * tools and ordinary error reporting: only persistence changes.
 */
export function statelessSession(request: ChatRequest): boolean {
  const value = request.options?.stateless_session;
  if (value !== undefined && typeof value !== "boolean") {
    throw new Error("Invalid stateless session option.");
  }
  if (value === true && request.options?.provider_session_id) {
    throw new Error("A stateless session cannot resume a provider session.");
  }
  return value === true;
}

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

export type StatusRequest = {
  command: "status";
  /** Only this runtime, so one check never starts the others. */
  provider?: ProviderId;
  /** Only these runtimes (all at once, under one deadline); never the rest. */
  providers?: ProviderId[];
};

export const PROVIDER_IDS: readonly ProviderId[] = ["codex", "claude", "opencode", "copilot"];

/**
 * Which runtimes a status request may start: `provider`, else `providers`,
 * else all of them. Unknown names are refused rather than ignored, so a
 * malformed scope can never widen to every runtime.
 */
export function statusScope(request: StatusRequest): ProviderId[] {
  const known = (value: unknown): value is ProviderId =>
    typeof value === "string" && (PROVIDER_IDS as readonly string[]).includes(value);
  if (request.provider !== undefined) {
    if (!known(request.provider)) throw new Error("Unknown provider.");
    return [request.provider];
  }
  if (request.providers !== undefined) {
    if (!Array.isArray(request.providers) || !request.providers.every(known)) {
      throw new Error("Unknown provider.");
    }
    return [...new Set(request.providers)];
  }
  return [...PROVIDER_IDS];
}

export type BrokerRequest =
  | StatusRequest
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
 * providers all issue id-shaped cursors, so anything outside that shape
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
