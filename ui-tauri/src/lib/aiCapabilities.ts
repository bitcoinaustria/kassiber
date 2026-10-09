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
  supports_reasoning_effort?: boolean;
  capabilities?: unknown;
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
  reasoning_efforts?: unknown;
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

/** Claude models that honour fast mode, mirrored from the broker. */
export const NATIVE_RUNTIME_FAST_MODELS: Partial<
  Record<NativeAiProviderRuntime, readonly string[]>
> = {
  claude: ["opus"],
};

export interface AiFastModeSupport {
  supported: boolean;
  /** Provider wording when it supplied one. */
  description?: string;
  /** Native runtime of the selected provider, for runtime-specific copy. */
  runtime: NativeAiProviderRuntime | null;
}

/**
 * Whether the selected model offers fast mode. Only an advertised capability
 * counts (a discovered row with `supports_fast_mode`, or the broker's fixed
 * Claude Opus entry); an unknown model fails closed, so fast mode is never
 * requested for a model that cannot honour it.
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
  return {
    supported: NATIVE_RUNTIME_FAST_MODELS[runtime]?.includes(selection.model) ?? false,
    runtime,
  };
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
  provider: "codex" | "claude" | "opencode";
  display_name: string;
  state: AiProviderRuntimeState;
  version?: string;
  message: string;
  privacy_posture: "remote";
  native_tools: "disabled";
  models: AiModelRow[];
}

export interface AiProviderRuntimeStatusData extends AiDiscoveryMetadata {
  providers: AiProviderRuntimeStatus[];
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
