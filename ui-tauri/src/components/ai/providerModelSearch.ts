import type {
  AiModelsListData,
  AiProviderRow,
  AiProviderRuntimeStatus,
} from "@/lib/aiCapabilities";
import {
  isNativeAiProviderLocator,
  nativeAiProviderRuntime,
} from "@/lib/aiCapabilities";

export function dedupeProviderRows(
  providers: AiProviderRow[],
  preferredProviderName?: string | null,
): AiProviderRow[] {
  const preferredByLocator = new Map<string, string>();
  for (const provider of providers) {
    const locator = provider.base_url.trim().toLowerCase();
    const runtime = nativeAiProviderRuntime(locator);
    if (!runtime) continue;
    if (preferredByLocator.has(locator)) continue;
    const sameLocator = providers.filter(
      (candidate) => candidate.base_url.trim().toLowerCase() === locator,
    );
    // The row the user picked wins, else the one named after the runtime.
    const pick =
      sameLocator.find((candidate) => candidate.name === preferredProviderName) ??
      sameLocator.find((candidate) => candidate.name === runtime) ??
      provider;
    preferredByLocator.set(locator, pick.name);
  }
  return providers.filter((provider) => {
    const locator = provider.base_url.trim().toLowerCase();
    const preferred = preferredByLocator.get(locator);
    return preferred === undefined || provider.name === preferred;
  });
}

export function filterModelRows(
  models: AiModelsListData["models"],
  query: string,
): AiModelsListData["models"] {
  const tokens = query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
  if (tokens.length === 0) return models;
  return models.filter((model) => {
    const haystack = [model.id, model.display_name, model.owned_by]
      .filter((value): value is string => typeof value === "string")
      .join(" ")
      .toLowerCase();
    return tokens.every((token) => haystack.includes(token));
  });
}

export function shouldPollProviderModels(provider: AiProviderRow): boolean {
  return (
    provider.kind === "local" &&
    !isNativeAiProviderLocator(provider.base_url)
  );
}

export function modelPrivacyPosture(
  provider: AiProviderRow,
  model: AiModelsListData["models"][number],
): AiProviderRow["kind"] {
  return model.privacy_posture ?? provider.kind;
}

/** Privacy-first row order: on-device, then TEE, then anything off-device. */
const POSTURE_RANK: Record<AiProviderRow["kind"], number> = {
  local: 0,
  tee: 1,
  remote: 2,
};

/**
 * Sort a provider's models by privacy posture, leaving the order inside each
 * posture untouched (`Array.sort` is stable) so the deliberate
 * default-model-first / selected-model-first placement survives.
 */
export function sortModelRowsByPosture(
  provider: AiProviderRow,
  models: AiModelsListData["models"],
): AiModelsListData["models"] {
  return [...models].sort(
    (a, b) =>
      POSTURE_RANK[modelPrivacyPosture(provider, a)] -
      POSTURE_RANK[modelPrivacyPosture(provider, b)],
  );
}

export function filterModelsByPrivacy(
  provider: AiProviderRow,
  models: AiModelsListData["models"],
  localOnly: boolean,
): AiModelsListData["models"] {
  return localOnly
    ? models.filter((model) => modelPrivacyPosture(provider, model) === "local")
    : models;
}

export function providerRuntimeSelectable(
  runtime: AiProviderRuntimeStatus | null | undefined,
): boolean {
  return runtime === null || runtime === undefined || runtime.state === "ready";
}

export function providerRuntimeTone(
  runtime: AiProviderRuntimeStatus | null | undefined,
): "ready" | "attention" | "unavailable" | "unknown" {
  if (!runtime) return "unknown";
  if (runtime.state === "ready") return "ready";
  if (runtime.state === "authentication_required") return "attention";
  return "unavailable";
}
