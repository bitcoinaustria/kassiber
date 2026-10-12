import * as React from "react";

import type {
  AssistantModelSelection,
  AssistantThinkingEffort,
} from "@/components/ai/assistantSession";
import { useProviderModelsSnapshot } from "@/components/ai/providerModelQuery";
import { useDaemon } from "@/daemon/client";
import {
  PROVIDER_DEFAULT_EFFORTS,
  isProviderDefaultEffort,
  selectedModelReasoningEfforts,
  type AiProvidersListData,
} from "@/lib/aiCapabilities";

/**
 * The offered level closest to `effort`, preferring the lower (cheaper) one
 * on a tie; "auto" when nothing Kassiber can label is offered.
 */
export function nearestOfferedEffort(
  effort: string,
  offered: readonly string[],
): AssistantThinkingEffort {
  const order: readonly string[] = PROVIDER_DEFAULT_EFFORTS;
  const target = order.indexOf(effort);
  let best: AssistantThinkingEffort = "auto";
  let bestDistance = Number.POSITIVE_INFINITY;
  order.forEach((level, index) => {
    if (!offered.includes(level)) return;
    const distance = Math.abs(index - target);
    if (distance < bestDistance) {
      best = level as AssistantThinkingEffort;
      bestDistance = distance;
    }
  });
  return best;
}

/**
 * Seed the session's reasoning effort from the selected provider's stored
 * default (Settings → AI providers, or `ai providers update --default-effort`).
 *
 * Like `kassiber chat`, a default only fills an effort the user left open: it
 * applies when a provider becomes selected and the effort is "auto" or came
 * from the previous provider's default. User selections must go through the
 * returned setter, which records that the level was chosen. A level the user picked stays theirs
 * across provider switches, unless the new provider's model does not offer
 * it; then the new provider's default, or else the nearest offered level,
 * takes over. Switching to a provider without a default drops an effort that
 * only came from the previous provider's default. Fast mode has no stored
 * default: it is billed at a higher rate and stays a per-session choice.
 */
export function useProviderDefaultEffort({
  selection,
  thinkingEffort,
  setThinkingEffort,
}: {
  selection: AssistantModelSelection | null;
  thinkingEffort: AssistantThinkingEffort;
  setThinkingEffort: (effort: AssistantThinkingEffort) => void;
}) {
  const providersQuery = useDaemon<AiProvidersListData>(
    "ai.providers.list",
    undefined,
    { meta: { shellProgress: false } },
  );
  const providers =
    providersQuery.data?.kind === "ai.providers.list"
      ? providersQuery.data.data?.providers
      : undefined;
  const providerName = selection?.provider ?? null;
  const row = providerName
    ? providers?.find((provider) => provider.name === providerName)
    : undefined;
  const storedDefault = isProviderDefaultEffort(row?.default_reasoning_effort)
    ? row.default_reasoning_effort
    : null;
  // Levels the selected model offers; empty when unknown (no Check models
  // yet and no fixed runtime list), which the picker treats as "all".
  const modelsSnapshot = useProviderModelsSnapshot(row);
  const offered = React.useMemo(
    () =>
      selectedModelReasoningEfforts({
        selection,
        providers: providers ?? [],
        models: modelsSnapshot?.models ?? [],
      }),
    [selection, providers, modelsSnapshot],
  );

  // Read inside the seeding effect without making every effort change re-run
  // it. Declared first, so they are current when that effect runs.
  const effortRef = React.useRef(thinkingEffort);
  const offeredRef = React.useRef(offered);
  React.useEffect(() => {
    effortRef.current = thinkingEffort;
    offeredRef.current = offered;
  }, [thinkingEffort, offered]);
  const seededForRef = React.useRef<string | null>(null);
  // Whether the last level set through the returned setter is the user's own
  // choice. Tracked apart from its value: re-picking the level a default
  // seeded is still a choice. "auto" is never a choice to keep (it is also the
  // starting state), so setting it leaves the flag alone: the picker resets
  // an unsupported level to auto through this same setter, in an effect that
  // runs before the seeding below, and that must not erase the user's pick.
  const chosenRef = React.useRef(false);

  const chooseThinkingEffort = React.useCallback(
    (effort: AssistantThinkingEffort) => {
      if (effort !== "auto") chosenRef.current = true;
      setThinkingEffort(effort);
    },
    [setThinkingEffort],
  );

  React.useEffect(() => {
    // Wait for the provider list, so a missing row means "no default", not
    // "not loaded yet".
    if (!providerName || !providers) return;
    if (seededForRef.current === providerName) return;
    seededForRef.current = providerName;
    const current = effortRef.current;
    const levels = offeredRef.current;
    const fits = (effort: string) => levels.length === 0 || levels.includes(effort);
    const chosen = chosenRef.current && current !== "auto";
    if (chosen && fits(current)) {
      // The user's level wins over any default, as `--reasoning-effort` does.
      return;
    }
    const target = storedDefault ?? (chosen ? current : null);
    if (target) {
      const next = fits(target) ? target : nearestOfferedEffort(target, levels);
      // A default may be replaced by the next provider's; the nearest level
      // to the user's own choice stays theirs.
      chosenRef.current = !storedDefault && next !== "auto";
      setThinkingEffort(next);
      return;
    }
    // No default here: a level that only came from a default goes back to auto.
    chosenRef.current = false;
    if (current !== "auto") setThinkingEffort("auto");
  }, [providerName, providers, storedDefault, setThinkingEffort]);

  return chooseThinkingEffort;
}
