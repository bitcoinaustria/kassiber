export type AssistantModelSelection = {
  provider: string;
  model: string;
} | null;

export type AiProviderKind = "local" | "remote" | "tee";

export interface AiProviderRow {
  name: string;
  base_url: string;
  kind: AiProviderKind;
  default_model?: string | null;
  notes?: string | null;
  acknowledged_at?: string | null;
  has_api_key: boolean;
  is_default: boolean;
  supports_reasoning_effort?: boolean;
  capabilities?: unknown;
}

export interface AiProvidersListData {
  providers: AiProviderRow[];
  default: string | null;
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
