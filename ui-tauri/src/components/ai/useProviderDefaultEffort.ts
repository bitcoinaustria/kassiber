import * as React from "react";

import type {
  AssistantModelSelection,
  AssistantThinkingEffort,
} from "@/components/ai/assistantSession";
import { useDaemon } from "@/daemon/client";
import {
  isProviderDefaultEffort,
  type AiProvidersListData,
} from "@/lib/aiCapabilities";

/**
 * Seed the session's reasoning effort from the selected provider's stored
 * default (Settings → AI providers, or `ai providers update --default-effort`).
 *
 * It applies when a provider becomes selected, never on later edits, so a
 * level the user picked in the composer stays theirs. Switching to a provider
 * without a default drops an effort that only came from the previous
 * provider's default. Fast mode has no stored default: it is billed at a
 * higher rate and stays a per-session choice.
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

  // Read inside the seeding effect without making every effort change re-run
  // it. Declared first, so it is current when that effect runs.
  const effortRef = React.useRef(thinkingEffort);
  React.useEffect(() => {
    effortRef.current = thinkingEffort;
  }, [thinkingEffort]);
  const seededForRef = React.useRef<string | null>(null);
  const seededEffortRef = React.useRef<AssistantThinkingEffort | null>(null);

  React.useEffect(() => {
    // Wait for the provider list, so a missing row means "no default", not
    // "not loaded yet".
    if (!providerName || !providers) return;
    if (seededForRef.current === providerName) return;
    seededForRef.current = providerName;
    if (storedDefault) {
      seededEffortRef.current = storedDefault;
      setThinkingEffort(storedDefault);
      return;
    }
    if (
      seededEffortRef.current !== null &&
      effortRef.current === seededEffortRef.current
    ) {
      setThinkingEffort("auto");
    }
    seededEffortRef.current = null;
  }, [providerName, providers, storedDefault, setThinkingEffort]);
}
