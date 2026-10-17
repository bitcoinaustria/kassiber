import type {
  AiModelRow,
  AiModelsListData,
  AiProviderRow,
  AiProviderRuntimeStatus,
} from "@/lib/aiCapabilities";
import { nativeAiProviderRuntime } from "@/lib/aiCapabilities";

function providerLocator(provider: AiProviderRow): string {
  return provider.base_url.trim().toLowerCase();
}

export function dedupeProviderRows(
  providers: AiProviderRow[],
  preferredProviderName?: string | null,
): AiProviderRow[] {
  const preferredByLocator = new Map<string, string>();
  for (const provider of providers) {
    const locator = providerLocator(provider);
    const runtime = nativeAiProviderRuntime(locator);
    if (!runtime) continue;
    if (preferredByLocator.has(locator)) continue;
    const sameLocator = providers.filter(
      (candidate) => providerLocator(candidate) === locator,
    );
    // The row the user picked wins, else the one named after the runtime.
    const pick =
      sameLocator.find((candidate) => candidate.name === preferredProviderName) ??
      sameLocator.find((candidate) => candidate.name === runtime) ??
      provider;
    preferredByLocator.set(locator, pick.name);
  }
  return providers.filter((provider) => {
    const preferred = preferredByLocator.get(providerLocator(provider));
    return preferred === undefined || provider.name === preferred;
  });
}

/**
 * Map each provider name `dedupeProviderRows` hid to the row it kept for the
 * same native CLI locator. Which alias is kept follows the current selection,
 * so a favorite saved under one alias must resolve to whichever one is shown.
 */
export function dedupedProviderAliases(
  providers: readonly AiProviderRow[],
  kept: readonly AiProviderRow[],
): Map<string, string> {
  const keptByLocator = new Map<string, string>();
  for (const provider of kept) {
    const locator = providerLocator(provider);
    if (nativeAiProviderRuntime(locator)) keptByLocator.set(locator, provider.name);
  }
  const aliases = new Map<string, string>();
  for (const provider of providers) {
    const keptName = keptByLocator.get(providerLocator(provider));
    if (keptName !== undefined && keptName !== provider.name) {
      aliases.set(provider.name, keptName);
    }
  }
  return aliases;
}

/**
 * Favorites as the picker shows them: each under the provider row that is
 * actually listed, once, in starring order.
 */
export function canonicalModelFavorites<T extends { provider: string; model: string }>(
  favorites: readonly T[],
  aliases: ReadonlyMap<string, string>,
): T[] {
  const seen = new Set<string>();
  const result: T[] = [];
  for (const favorite of favorites) {
    const provider = aliases.get(favorite.provider) ?? favorite.provider;
    const key = JSON.stringify([provider, favorite.model]);
    if (seen.has(key)) continue;
    seen.add(key);
    result.push(provider === favorite.provider ? favorite : { ...favorite, provider });
  }
  return result;
}

/**
 * The stored favorites a shown row stands for: its own entry and any saved
 * under a hidden alias of the same provider. Each distinct entry once.
 */
export function storedModelFavoritesFor<T extends { provider: string; model: string }>(
  favorites: readonly T[],
  aliases: ReadonlyMap<string, string>,
  provider: string,
  model: string,
): T[] {
  const seen = new Set<string>();
  return favorites.filter((favorite) => {
    if (favorite.model !== model) return false;
    if ((aliases.get(favorite.provider) ?? favorite.provider) !== provider) return false;
    const key = JSON.stringify([favorite.provider, favorite.model]);
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
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

/**
 * Marks in a model id that name something other than a chat model:
 * embeddings, speech in or out, image generation, moderation, and realtime or
 * audio models. Deliberately few and specific, so no chat model is hidden by
 * its name; they apply only where a listing says nothing about the model.
 */
const NON_CHAT_MODEL_ID_PATTERNS: readonly RegExp[] = [
  /embed/, // text-embedding-3-small, nomic-embed-text
  /(?:^|[^a-z])tts(?:[^a-z]|$)/, // tts-1, gpt-4o-mini-tts
  /whisper/, // whisper-1
  /dall-e/, // dall-e-3
  /gpt-image/, // gpt-image-1
  /moderation/, // omni-moderation-latest
  /transcribe/, // gpt-4o-transcribe
  /realtime/, // gpt-realtime, gpt-4o-realtime-preview
  /(?:^|[^a-z])audio(?:[^a-z]|$)/, // gpt-audio, gpt-4o-audio-preview
];

export function modelIdNamesNonChatModel(id: string): boolean {
  const lowered = id.toLowerCase();
  return NON_CHAT_MODEL_ID_PATTERNS.some((pattern) => pattern.test(lowered));
}

/**
 * Whether a model an HTTP endpoint listed can run the assistant, judged only
 * from the cached listing (nothing is asked). The assistant needs tool calls
 * and a text answer: a listing that says the model takes no tools, or that
 * its output has no text, rules it out. Where a listing says nothing, as
 * OpenAI's does, only an id with a clear non-chat mark is ruled out.
 */
export function httpModelRunsAssistant(model: AiModelRow): boolean {
  if (model.supports_tools === false) return false;
  const outputs = Array.isArray(model.output_modalities) ? model.output_modalities : [];
  if (outputs.length > 0) return outputs.includes("text");
  if (model.supports_tools === true) return true;
  return !modelIdNamesNonChatModel(model.id);
}
