import * as React from "react";

import type {
  AssistantModelSelection,
  AssistantThinkingEffort,
} from "@/components/ai/assistantSession";
import { useProviderModelsSnapshot } from "@/components/ai/providerModelQuery";
import { useDaemon } from "@/daemon/client";
import { useUiStore } from "@/store/ui";
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
 * returned setter, which records that the level was chosen. A level the user
 * picked stays theirs across provider switches, unless the new provider's
 * model does not offer it; then the new provider's default, or else the
 * nearest offered level, takes over. Switching to a provider without a
 * default drops an effort that only came from the previous provider's
 * default.
 *
 * Providers and their defaults belong to a book, so a book switch (a new
 * daemon session) seeds again from the new book's provider row even when the
 * same provider stays selected, and a level chosen in the old book does not
 * carry over: the same name can be a differently configured provider there.
 * Fast mode has no stored default: it is billed at a higher rate and stays a
 * per-session choice.
 *
 * Returns the setter for user picks and `effortSeeded`, false while the
 * current book's provider list has not been seeded yet.
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
  const dataMode = useUiStore((state) => state.dataMode);
  const daemonSession = useUiStore((state) => state.daemonSession);
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
  // The book session and provider the effort was last seeded for, and the
  // levels known then ("" while they were unknown).
  const seededForRef = React.useRef<{
    scope: string;
    provider: string;
    levels: string;
    /** The list had failed to load: no default was known, so seed again once it arrives. */
    withoutList?: boolean;
  } | null>(null);
  // Changes when Check models resolves (or replaces) what the model offers,
  // so the fallback below runs again with the real list.
  const offeredKey = offered.join(",");
  // Whether the last level set through the returned setter is the user's own
  // choice. Tracked apart from its value: re-picking the level a default
  // seeded is still a choice. "auto" is never a choice to keep (it is also the
  // starting state), so setting it leaves the flag alone: the picker resets
  // an unsupported level to auto through this same setter, in an effect that
  // runs before the seeding below, and that must not erase the user's pick.
  //
  // This hook owns the correction of an unsupported level; the picker's reset
  // is only a safety net. The picker is a descendant of the session provider,
  // so in one commit its effect runs first and this one last: whatever level
  // this sets is the one that stays, deterministically.
  const chosenRef = React.useRef(false);
  // The book session and provider whose seeding has run. Until it matches the
  // current one (the provider list is still loading, say after a book
  // switch), the live effort may be the previous book's or plain auto, so the
  // session holds sends back rather than send them with a wrong effort.
  const [seededKey, setSeededKey] = React.useState<string | null>(null);
  const seedKey = providerName ? `${dataMode}:${daemonSession}:${providerName}` : null;
  // A list that fails to load still counts once the error path below has
  // run for this scope (it resets the effort), so sends are never held for good.
  const effortSeeded = seedKey === null || seededKey === seedKey;
  const listFailed = providersQuery.isError === true && !providers;

  // A level chosen in one book must not survive into the next, but a level
  // picked in the new book while its provider list loads is that book's
  // choice. So the flag is cleared when the book changes, not when seeding
  // later runs. Declared before the seeding effect, so it runs first.
  const scope = `${dataMode}:${daemonSession}`;
  const scopeRef = React.useRef(scope);
  React.useEffect(() => {
    if (scopeRef.current === scope) return;
    scopeRef.current = scope;
    chosenRef.current = false;
  }, [scope]);

  const chooseThinkingEffort = React.useCallback(
    (effort: AssistantThinkingEffort) => {
      if (effort !== "auto") chosenRef.current = true;
      setThinkingEffort(effort);
    },
    [setThinkingEffort],
  );

  React.useEffect(() => {
    if (!providerName) return;
    const seededFor = seededForRef.current;
    const sameSeed =
      seededFor?.scope === scope &&
      seededFor.provider === providerName &&
      !seededFor.withoutList;
    if (!providers) {
      // Wait for the provider list, so a missing row means "no default", not
      // "not loaded yet". If it failed to load, release sends with the safe
      // effort instead: auto, unless the user picked a level in this book;
      // never the previous book's level.
      if (!listFailed) return;
      if (seededFor?.scope === scope && seededFor.provider === providerName) return;
      seededForRef.current = {
        scope,
        provider: providerName,
        levels: offeredKey,
        withoutList: true,
      };
      setSeededKey(`${scope}:${providerName}`);
      if (chosenRef.current && effortRef.current !== "auto") return;
      chosenRef.current = false;
      if (effortRef.current !== "auto") setThinkingEffort("auto");
      return;
    }
    if (sameSeed && seededFor.levels === offeredKey) return;
    seededForRef.current = { scope, provider: providerName, levels: offeredKey };
    // Lands in the same render as any effort the seeding below sets.
    setSeededKey(`${scope}:${providerName}`);
    const current = effortRef.current;
    const levels = offeredRef.current;
    const fits = (effort: string) => levels.length === 0 || levels.includes(effort);
    if (sameSeed) {
      // Same provider, but what its model offers just became known (or
      // changed): only a level it no longer offers needs the fallback. "auto"
      // and any supported level, chosen or seeded, stay as they are.
      if (current === "auto" || fits(current)) return;
    }
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
  }, [
    listFailed,
    offeredKey,
    providerName,
    providers,
    scope,
    storedDefault,
    setThinkingEffort,
  ]);

  return { setThinkingEffort: chooseThinkingEffort, effortSeeded };
}
