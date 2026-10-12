import * as React from "react";

import { useCachedProviderModels } from "@/components/ai/providerModelQuery";
import { useDaemon } from "@/daemon/client";
import {
  selectedModelFastMode,
  type AiFastModeSupport,
  type AiProviderRow,
  type AiProvidersListData,
  type AssistantModelSelection,
} from "@/lib/aiCapabilities";

export interface FastModeSupport extends AiFastModeSupport {
  /** The provider list has loaded, so `supported: false` is an answer. */
  resolved: boolean;
}

/**
 * Whether the selected model offers fast mode, from the provider list and any
 * model inventory already in the query cache. Like reasoning support, this
 * never starts discovery: an undiscovered model reads as unsupported, so fast
 * mode fails closed until Check models has read the installed CLI's version.
 */
export function useFastModeSupport(
  selection: AssistantModelSelection,
  enabled = true,
): FastModeSupport {
  const providersQuery = useDaemon<AiProvidersListData>(
    "ai.providers.list",
    undefined,
    { enabled, meta: { shellProgress: false } },
  );
  const providersData =
    providersQuery.data?.kind === "ai.providers.list"
      ? providersQuery.data.data
      : null;
  const providers = React.useMemo<AiProviderRow[]>(
    () => providersData?.providers ?? [],
    [providersData],
  );
  const selectedProvider = selection
    ? providers.find((provider) => provider.name === selection.provider)
    : undefined;
  // Never starts discovery: reads only what Check models found for the
  // selected provider's current endpoint.
  const modelsQuery = useCachedProviderModels(selectedProvider);
  const modelsData =
    modelsQuery.data?.kind === "ai.list_models" ? modelsQuery.data.data : null;
  const models = React.useMemo(() => modelsData?.models ?? [], [modelsData]);
  const support = selectedModelFastMode({ selection, providers, models });
  return { ...support, resolved: Boolean(providersData) };
}
