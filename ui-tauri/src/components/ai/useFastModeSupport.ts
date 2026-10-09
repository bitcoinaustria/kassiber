import * as React from "react";

import { useDaemon } from "@/daemon/client";
import {
  selectedModelFastMode,
  type AiFastModeSupport,
  type AiModelsListData,
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
 * never starts discovery: an undiscovered model reads as unsupported (except
 * the broker's fixed Claude Opus entry), so fast mode fails closed.
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
  const modelsQuery = useDaemon<AiModelsListData>(
    "ai.list_models",
    selectedProvider ? { provider: selectedProvider.name } : undefined,
    {
      // Cache read only: discovery may contact the provider.
      enabled: false,
      staleTime: 5 * 60 * 1000,
      meta: { shellProgress: false },
    },
  );
  const modelsData =
    modelsQuery.data?.kind === "ai.list_models" ? modelsQuery.data.data : null;
  const models = React.useMemo(() => modelsData?.models ?? [], [modelsData]);
  const support = selectedModelFastMode({ selection, providers, models });
  return { ...support, resolved: Boolean(providersData) };
}
