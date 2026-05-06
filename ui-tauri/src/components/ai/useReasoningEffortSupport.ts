import * as React from "react";

import type { AssistantThinkingEffort } from "@/components/ai/assistantSession";
import { useDaemon } from "@/daemon/client";
import {
  providerSupportsReasoningEffort,
  selectedModelSupportsReasoningEffort,
  type AiModelsListData,
  type AiProviderRow,
  type AiProvidersListData,
  type AssistantModelSelection,
} from "@/lib/aiCapabilities";

export interface ReasoningEffortSupport {
  supported: boolean;
  resolved: boolean;
}

export function useReasoningEffortSupport(
  selection: AssistantModelSelection,
  enabled = true,
): ReasoningEffortSupport {
  const providersQuery = useDaemon<AiProvidersListData>(
    "ai.providers.list",
    undefined,
    { enabled },
  );
  const providersData =
    providersQuery.data?.kind === "ai.providers.list"
      ? providersQuery.data.data
      : null;
  const hasProvidersResponse = Boolean(providersData);
  const providers = React.useMemo<AiProviderRow[]>(
    () => providersData?.providers ?? [],
    [providersData],
  );

  const selectedProvider = selection
    ? providers.find((provider) => provider.name === selection.provider)
    : undefined;
  const modelsQuery = useDaemon<AiModelsListData>(
    "ai.list_models",
    selectedProvider ? { provider: selectedProvider.name } : undefined,
    {
      enabled: enabled && Boolean(selectedProvider),
      staleTime: 5 * 60 * 1000,
    },
  );
  const modelsData =
    modelsQuery.data?.kind === "ai.list_models" ? modelsQuery.data.data : null;
  const hasModelsResponse = Boolean(modelsData);
  const models = React.useMemo(
    () => modelsData?.models ?? [],
    [modelsData],
  );

  const providerSupported = providerSupportsReasoningEffort(selectedProvider);
  const supported = selectedModelSupportsReasoningEffort({
    selection,
    providers,
    models,
  });
  const resolved =
    !selection ||
    providerSupported ||
    providersQuery.isError ||
    (Boolean(hasProvidersResponse) &&
      (!selectedProvider || modelsQuery.isError || Boolean(hasModelsResponse)));

  return { supported, resolved };
}

export function useSupportedReasoningEffort({
  selection,
  thinkingEffort,
  setThinkingEffort,
  enabled = true,
}: {
  selection: AssistantModelSelection;
  thinkingEffort: AssistantThinkingEffort;
  setThinkingEffort: (effort: AssistantThinkingEffort) => void;
  enabled?: boolean;
}): boolean {
  const support = useReasoningEffortSupport(selection, enabled);

  React.useEffect(() => {
    if (support.resolved && !support.supported && thinkingEffort !== "auto") {
      setThinkingEffort("auto");
    }
  }, [
    support.resolved,
    support.supported,
    thinkingEffort,
    setThinkingEffort,
  ]);

  return support.supported;
}
