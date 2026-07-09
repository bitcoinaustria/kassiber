export type AssistantModelSelection = {
  provider: string;
  model: string;
} | null;

export type AiProviderKind = "local" | "remote" | "tee";
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
  owned_by?: string;
  supports_reasoning_effort?: boolean;
  supported_parameters?: unknown;
  reasoning_efforts?: unknown;
  capabilities?: unknown;
}

export interface AiModelsListData {
  provider: string;
  models: AiModelRow[];
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
  return [...new Set(advertised)];
}
