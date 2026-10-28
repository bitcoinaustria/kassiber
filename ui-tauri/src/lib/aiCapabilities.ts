export type AssistantModelSelection = {
  provider: string;
  model: string;
} | null;

export type AiProviderKind = "local" | "remote" | "tee";
export const NATIVE_AI_PROVIDER_BY_LOCATOR = {
  "claude-cli://default": "claude",
  "codex-cli://default": "codex",
  "opencode-cli://default": "opencode",
  // Agent Client Protocol agents behind the broker's generic ACP adapter.
  "copilot-cli://default": "copilot",
} as const;
export type NativeAiProviderRuntime =
  (typeof NATIVE_AI_PROVIDER_BY_LOCATOR)[keyof typeof NATIVE_AI_PROVIDER_BY_LOCATOR];

const NATIVE_RUNTIME_LOOKUP: Record<string, NativeAiProviderRuntime> =
  NATIVE_AI_PROVIDER_BY_LOCATOR;

export function nativeAiProviderRuntime(value: unknown): NativeAiProviderRuntime | null {
  if (typeof value !== "string") return null;
  return NATIVE_RUNTIME_LOOKUP[value.trim().toLowerCase()] ?? null;
}

export function isNativeAiProviderLocator(value: unknown): boolean {
  return nativeAiProviderRuntime(value) !== null;
}

/**
 * Whether an HTTP(S) base URL names this device: `localhost`, an IPv4 address
 * in 127.0.0.0/8, or an IPv6 loopback (`::1`, or 127/8 mapped into IPv6).
 * Mirrors the daemon's `_is_loopback_ai_http_url`, which reads the host as
 * written rather than through a browser's URL normalisation, so shorthand
 * such as `127.1` or `0x7f000001` is not loopback here either.
 */
export function isLoopbackAiHttpUrl(raw: string): boolean {
  const authority = /^https?:\/\/([^/?#]*)/.exec(raw.trim())?.[1];
  if (authority === undefined) return false;
  const hostinfo = authority.slice(authority.lastIndexOf("@") + 1);
  let host: string;
  if (hostinfo.startsWith("[")) {
    const end = hostinfo.indexOf("]");
    if (end < 0) return false;
    host = hostinfo.slice(1, end);
  } else {
    host = hostinfo.split(":")[0] ?? "";
  }
  host = host.toLowerCase();
  if (host === "localhost") return true;
  const ipv4 = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(host);
  if (ipv4) {
    const octets = ipv4.slice(1);
    // Like Python's ipaddress: no leading zeros, and each octet at most 255.
    if (octets.some((octet) => (octet.length > 1 && octet.startsWith("0")) || Number(octet) > 255)) {
      return false;
    }
    return octets[0] === "127";
  }
  if (!host.includes(":")) return false;
  let canonical: string;
  try {
    // The URL parser only canonicalises the IPv6 spelling here.
    canonical = new URL(`http://[${host}]/`).hostname;
  } catch {
    return false;
  }
  return canonical === "[::1]" || /^\[::ffff:7f[0-9a-f]{2}:[0-9a-f]{1,4}\]$/.test(canonical);
}

export type AiSecretStoreId =
  | "macos_keychain"
  | "windows_dpapi"
  | "linux_secret_service"
  | "sqlcipher_inline";
export type AiSecretRefState =
  | "ok"
  | "missing"
  | "needs_reauth"
  | "unavailable";

export interface AiProviderSecretRef {
  store_id: AiSecretStoreId;
  state: AiSecretRefState;
}

/**
 * OpenAI wire format of an HTTP provider. "auto" speaks the detected format,
 * else the Responses API, and falls back to Chat Completions once when the
 * server has no /responses.
 */
export type AiApiFormat = "auto" | "responses" | "chat_completions";
export type AiWireApiFormat = Exclude<AiApiFormat, "auto">;

export interface AiProviderRow {
  name: string;
  display_name?: string | null;
  base_url: string;
  kind: AiProviderKind;
  default_model?: string | null;
  notes?: string | null;
  acknowledged_at?: string | null;
  has_api_key: boolean;
  secret_ref?: AiProviderSecretRef;
  is_default: boolean;
  /**
   * The user's switch: a disabled provider is hidden from the model picker
   * and refused by the daemon for chat. Absent on older daemons = enabled.
   */
  enabled?: boolean;
  /**
   * When the provider was last checked at its current address, kept across
   * restarts; null when never. Only a check the user started sets it.
   */
  last_checked_at?: string | null;
  /**
   * False for a built-in provider the user has not used yet (not the default,
   * never checked, never acknowledged). Settings folds those away; it never
   * decides whether the provider may be contacted. Absent = set up.
   */
  set_up?: boolean;
  /** Effort a new session starts with for this provider; null = auto. */
  default_reasoning_effort?: string | null;
  /** The user's wire-format setting; always "auto" for a CLI agent. */
  api_format?: AiApiFormat;
  /**
   * The format the endpoint was last seen to answer (connection test or chat
   * fallback); null until detected and after the base URL changes.
   */
  detected_api_format?: AiWireApiFormat | null;
  /** The format the next chat speaks; null for a CLI agent. */
  effective_api_format?: AiWireApiFormat | null;
  /**
   * The user's explicit choice to send this HTTP provider's Chat Completions
   * requests without `store: false`; its own retention then applies.
   */
  omit_store_field?: boolean;
  supports_reasoning_effort?: boolean;
  capabilities?: unknown;
}

/**
 * Levels a provider's stored default effort may take; mirrors the daemon's
 * `AI_REASONING_EFFORTS`. "auto" is the absence of a default.
 */
export const PROVIDER_DEFAULT_EFFORTS = [
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
] as const;
export type ProviderDefaultEffort = (typeof PROVIDER_DEFAULT_EFFORTS)[number];

/**
 * The levels an OpenAI-compatible HTTP endpoint takes; mirrors the daemon's
 * `HTTP_REASONING_EFFORTS`. Kassiber sends its higher levels as "high".
 */
export const HTTP_REASONING_EFFORTS = ["low", "medium", "high"] as const;

function isHttpReasoningEffort(value: string): boolean {
  return (HTTP_REASONING_EFFORTS as readonly string[]).includes(value);
}

export function isProviderDefaultEffort(value: unknown): value is ProviderDefaultEffort {
  return (
    typeof value === "string" &&
    (PROVIDER_DEFAULT_EFFORTS as readonly string[]).includes(value)
  );
}

/** Whether a provider may be offered for chat. */
export function providerEnabled(provider: Pick<AiProviderRow, "enabled">): boolean {
  return provider.enabled !== false;
}

export interface AiProvidersListData {
  providers: AiProviderRow[];
  default: string | null;
  secret_store_policy?: {
    platform?: "macos" | "windows" | "linux" | "unsupported";
    default?: {
      store_id: AiSecretStoreId;
      native_store_id?: AiSecretStoreId | null;
      native_available: boolean;
      warning?: string | null;
    };
  };
}

export interface AiModelRow {
  id: string;
  display_name?: string;
  owned_by?: string;
  source_provider?: string;
  privacy_posture?: AiProviderKind;
  privacy_reason?: string;
  supports_reasoning_effort?: boolean;
  supported_parameters?: unknown;
  /**
   * Effort levels the model takes. For an HTTP model these are the wire
   * levels (some of low, medium, high); higher levels are sent as high.
   */
  reasoning_efforts?: unknown;
  /** HTTP models: whether the listing says the model takes tools; null = not said. */
  supports_tools?: boolean | null;
  /** HTTP models: OpenRouter's architecture modalities, lowercase. */
  input_modalities?: string[];
  output_modalities?: string[];
  capabilities?: unknown;
  /** Faster, higher-cost serving mode (Codex Fast tier, Claude Opus fast mode). */
  supports_fast_mode?: boolean;
  /** Provider wording for that mode, e.g. "2x speed, increased usage". */
  fast_mode_description?: string;
}

/**
 * What the broker advertises for native CLI runtimes whose answer does not
 * depend on the model (provider-broker `CLAUDE_MODELS` and Copilot's
 * `efforts`). Used only when the selected model's row is not known yet —
 * before **Check models**, the picker holds just the stored default id — so
 * the effort menu still offers the real levels without contacting anything.
 * Codex and OpenCode vary per model and stay discovery-only.
 */
export const NATIVE_RUNTIME_REASONING_EFFORTS: Partial<
  Record<NativeAiProviderRuntime, readonly string[]>
> = {
  claude: ["low", "medium", "high", "xhigh", "max"],
  copilot: ["low", "medium", "high", "xhigh", "max"],
};

export interface AiFastModeSupport {
  supported: boolean;
  /** Provider wording when it supplied one. */
  description?: string;
  /** Native runtime of the selected provider, for runtime-specific copy. */
  runtime: NativeAiProviderRuntime | null;
}

/**
 * Whether the selected model offers fast mode. Only a discovered row with
 * `supports_fast_mode` counts: the broker decides it per installed CLI version
 * (Claude Code 2.1.205, Codex 0.151.0), so there is no fixed fallback, and an
 * undiscovered model fails closed until Check models has run.
 */
export function selectedModelFastMode({
  selection,
  providers,
  models,
}: {
  selection: AssistantModelSelection;
  providers: AiProviderRow[];
  models: AiModelRow[];
}): AiFastModeSupport {
  if (!selection) return { supported: false, runtime: null };
  const provider = providers.find((row) => row.name === selection.provider);
  const runtime = provider ? nativeAiProviderRuntime(provider.base_url) : null;
  // Fast mode exists only behind the provider broker.
  if (!runtime) return { supported: false, runtime: null };
  const model = models.find((row) => row.id === selection.model);
  if (model && typeof model.supports_fast_mode === "boolean") {
    const description =
      typeof model.fast_mode_description === "string" &&
      model.fast_mode_description.trim()
        ? model.fast_mode_description.trim()
        : undefined;
    return model.supports_fast_mode
      ? { supported: true, runtime, ...(description ? { description } : {}) }
      : { supported: false, runtime };
  }
  return { supported: false, runtime };
}

export interface AiDiscoveryMetadata {
  checked_at?: string | null;
  stale?: boolean;
  error?: {
    code: string;
    message: string;
  } | null;
}

export interface AiModelsListData extends AiDiscoveryMetadata {
  provider: string;
  models: AiModelRow[];
}

export type AiProviderRuntimeState =
  | "ready"
  | "missing_executable"
  | "authentication_required"
  | "error";

export interface AiProviderRuntimeStatus {
  provider: NativeAiProviderRuntime;
  display_name: string;
  /** Basename of the binary that answered; never a full path. */
  executable?: string;
  state: AiProviderRuntimeState;
  version?: string;
  message: string;
  privacy_posture: "remote";
  native_tools: "disabled";
  models: AiModelRow[];
}

export interface AiProviderRuntimeStatusData extends AiDiscoveryMetadata {
  providers: AiProviderRuntimeStatus[];
  /** Set when the request was scoped to one configured provider. */
  provider?: string;
}

function hasTruthyCapability(value: unknown): boolean {
  return value === true || value === "true" || value === "supported";
}

function listIncludesString(value: unknown, target: string): boolean {
  return Array.isArray(value) && value.some((item) => item === target);
}

function hasNonEmptyStringList(value: unknown): boolean {
  return Array.isArray(value) && value.some((item) => typeof item === "string");
}

function capabilityObjectSupportsReasoningEffort(value: unknown): boolean {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const record = value as Record<string, unknown>;
  return (
    hasTruthyCapability(record.reasoning_effort) ||
    listIncludesString(record.supported_parameters, "reasoning_effort") ||
    hasNonEmptyStringList(record.reasoning_efforts)
  );
}

export function providerSupportsReasoningEffort(
  provider: AiProviderRow | null | undefined,
): boolean {
  if (!provider) return false;
  return (
    provider.supports_reasoning_effort === true ||
    capabilityObjectSupportsReasoningEffort(provider.capabilities)
  );
}

export function modelSupportsReasoningEffort(
  model: AiModelRow | null | undefined,
): boolean {
  if (!model) return false;
  return (
    model.supports_reasoning_effort === true ||
    listIncludesString(model.supported_parameters, "reasoning_effort") ||
    hasNonEmptyStringList(model.reasoning_efforts) ||
    capabilityObjectSupportsReasoningEffort(model.capabilities)
  );
}

export function selectedModelSupportsReasoningEffort({
  selection,
  providers,
  models,
}: {
  selection: AssistantModelSelection;
  providers: AiProviderRow[];
  models: AiModelRow[];
}): boolean {
  if (!selection) return false;
  const provider = providers.find((row) => row.name === selection.provider);
  if (providerSupportsReasoningEffort(provider)) return true;
  const model = models.find((row) => row.id === selection.model);
  if (provider && !nativeAiProviderRuntime(provider.base_url)) {
    // The daemon reads an HTTP listing (OpenRouter's `reasoning`, a
    // `reasoning_effort` parameter) into this flag; a model it says nothing
    // about, or says no about, takes no effort.
    return model?.supports_reasoning_effort === true;
  }
  return modelSupportsReasoningEffort(model);
}

function normalizeEffortList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter(
    (item): item is string => typeof item === "string" && item.trim().length > 0,
  );
}

function reasoningEffortsFromCapabilityObject(value: unknown): string[] {
  if (!value || typeof value !== "object" || Array.isArray(value)) return [];
  return normalizeEffortList(
    (value as Record<string, unknown>).reasoning_efforts,
  );
}

/**
 * The specific reasoning-effort levels a model (or its provider) advertises,
 * lower-cased and de-duplicated in advertised order. Empty when nothing is
 * advertised — callers should fall back to their default level set.
 *
 * An HTTP model offers only the wire levels its listing names
 * (`reasoning_efforts`, which the daemon narrows to low/medium/high), or all
 * three when it takes an effort without naming levels; one that takes no
 * effort, or is not checked yet, offers none.
 */
export function selectedModelReasoningEfforts({
  selection,
  providers,
  models,
}: {
  selection: AssistantModelSelection;
  providers: AiProviderRow[];
  models: AiModelRow[];
}): string[] {
  if (!selection) return [];
  const model = models.find((row) => row.id === selection.model);
  const provider = providers.find((row) => row.name === selection.provider);
  if (provider && !nativeAiProviderRuntime(provider.base_url)) {
    if (model?.supports_reasoning_effort !== true) return [];
    const listed = normalizeEffortList(model.reasoning_efforts)
      .map((effort) => effort.toLowerCase())
      .filter(isHttpReasoningEffort);
    return listed.length > 0 ? [...new Set(listed)] : [...HTTP_REASONING_EFFORTS];
  }
  const advertised = [
    ...(model ? normalizeEffortList(model.reasoning_efforts) : []),
    ...(model ? reasoningEffortsFromCapabilityObject(model.capabilities) : []),
    ...(provider ? reasoningEffortsFromCapabilityObject(provider.capabilities) : []),
  ].map((effort) => effort.toLowerCase());
  if (advertised.length === 0 && provider && !model?.reasoning_efforts) {
    // The model row is not known yet (no Check models); fall back to the
    // runtime's fixed levels where the broker publishes one.
    const runtime = nativeAiProviderRuntime(provider.base_url);
    const fixed = runtime ? NATIVE_RUNTIME_REASONING_EFFORTS[runtime] : undefined;
    if (fixed) return [...fixed];
  }
  return [...new Set(advertised)];
}
