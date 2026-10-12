import type {
  AiModelsListData,
  AiProviderRow,
  AiProviderRuntimeStatus,
} from "@/lib/aiCapabilities";
import { nativeAiProviderRuntime } from "@/lib/aiCapabilities";

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
 * Stable privacy-first sort: on-device, then TEE, then off-device, leaving the
 * order inside each posture untouched (`Array.sort` is stable).
 */
export function sortByPrivacyPosture<T>(
  items: readonly T[],
  posture: (item: T) => AiProviderRow["kind"],
): T[] {
  return [...items].sort((a, b) => POSTURE_RANK[posture(a)] - POSTURE_RANK[posture(b)]);
}

/**
 * Sort a provider's models by privacy posture, so the deliberate
 * default-model-first / selected-model-first placement survives inside each
 * posture.
 */
export function sortModelRowsByPosture(
  provider: AiProviderRow,
  models: AiModelsListData["models"],
): AiModelsListData["models"] {
  return sortByPrivacyPosture(models, (model) => modelPrivacyPosture(provider, model));
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
