/**
 * Two-level provider/model picker for the AI chat input.
 *
 * A small badge per row makes the `local` / `remote` / `tee` distinction
 * visible — the privacy posture in docs/reference/ai.md depends on the user
 * being able to tell at a glance whether a prompt is about to leave the
 * device.
 */

import * as React from "react";
import { useQueries } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";

import {
  ModelSelector,
  ModelSelectorContent,
  ModelSelectorEmpty,
  ModelSelectorGroup,
  ModelSelectorItem,
  ModelSelectorLabel,
  ModelSelectorName,
  ModelSelectorTrigger,
  ModelSelectorValue,
} from "@/components/ai-elements";
import { Button } from "@/components/ui/button";
import {
  DaemonRequestError,
  daemonQueryKey,
  useDaemon,
} from "@/daemon/client";
import { getTransport, type DaemonEnvelope } from "@/daemon/transport";
import type {
  AiProviderKind,
  AiModelsListData,
  AiProviderRow,
  AiProvidersListData,
} from "@/lib/aiCapabilities";
import { useUiStore, type DataMode } from "@/store/ui";
import { cn } from "@/lib/utils";

interface ProviderModelPickerProps {
  value: { provider: string; model: string } | null;
  onChange: (next: { provider: string; model: string } | null) => void;
  enabled?: boolean;
  onActiveProviderKindChange?: (kind: AiProviderKind | null) => void;
}

function rowValue(provider: string, model: string): string {
  return `${provider}::${model}`;
}

function parseRowValue(value: string): { provider: string; model: string } | null {
  const idx = value.indexOf("::");
  if (idx < 0) return null;
  return { provider: value.slice(0, idx), model: value.slice(idx + 2) };
}

function isCliProvider(provider: AiProviderRow): boolean {
  return (
    provider.base_url === "claude-cli://default" ||
    provider.base_url === "codex-cli://default"
  );
}

async function fetchProviderModels(
  dataMode: DataMode,
  provider: string,
): Promise<DaemonEnvelope<AiModelsListData>> {
  const envelope = await getTransport(dataMode).invoke<AiModelsListData>({
    kind: "ai.list_models",
    args: { provider },
  });
  if (envelope.kind === "error" || envelope.error) {
    throw new DaemonRequestError("ai.list_models", envelope);
  }
  return envelope;
}

export function ProviderModelPicker({
  value,
  onChange,
  enabled = true,
  onActiveProviderKindChange,
}: ProviderModelPickerProps) {
  const dataMode = useUiStore((state) => state.dataMode);
  const daemonSession = useUiStore((state) => state.daemonSession);
  const providersQuery = useDaemon<AiProvidersListData>(
    "ai.providers.list",
    undefined,
    { enabled },
  );
  const providers = React.useMemo<AiProviderRow[]>(
    () =>
      providersQuery.data?.kind === "ai.providers.list" &&
      providersQuery.data.data
        ? providersQuery.data.data.providers
        : [],
    [providersQuery.data],
  );

  // Resolve the active provider eagerly so the models query fires for the
  // default provider even before the parent has picked a `value`. Without
  // this, a freshly-seeded `ollama` row (which has `default_model = null`)
  // would leave the picker showing only disabled placeholders, blocking
  // the first chat send.
  const fallbackProvider = React.useMemo(
    () => providers.find((p) => p.is_default) ?? providers[0],
    [providers],
  );
  const selectedProvider = value
    ? providers.find((p) => p.name === value.provider)
    : fallbackProvider;

  React.useEffect(() => {
    onActiveProviderKindChange?.(selectedProvider?.kind ?? null);
  }, [onActiveProviderKindChange, selectedProvider?.kind]);

  const modelQueries = useQueries({
    queries: providers.map((provider) => ({
      queryKey: daemonQueryKey(
        dataMode,
        daemonSession,
        "ai.list_models",
        { provider: provider.name },
      ),
      queryFn: () => fetchProviderModels(dataMode, provider.name),
      enabled,
      refetchOnMount: isCliProvider(provider) ? "always" : false,
      staleTime: 5 * 60 * 1000,
    })),
  });
  const modelsByProvider = React.useMemo(() => {
    const next = new Map<string, AiModelsListData["models"]>();
    providers.forEach((provider, index) => {
      const result = modelQueries[index];
      next.set(
        provider.name,
        result?.data?.kind === "ai.list_models" && result.data.data
          ? result.data.data.models
          : [],
      );
    });
    return next;
  }, [providers, modelQueries]);
  const models = selectedProvider
    ? (modelsByProvider.get(selectedProvider.name) ?? [])
    : [];

  // Once providers (and, if needed, models) land, seed a selection so the
  // user can send a chat without first opening Settings. Prefer the saved
  // `default_model`; otherwise pick the first model the provider advertises.
  React.useEffect(() => {
    if (!enabled) return;
    if (
      value &&
      selectedProvider &&
      isCliProvider(selectedProvider) &&
      value.provider === selectedProvider.name &&
      value.model === "default" &&
      models.length > 0 &&
      models[0].id !== "default"
    ) {
      onChange({
        provider: selectedProvider.name,
        model: models[0].id,
      });
      return;
    }
    if (value || !fallbackProvider) return;
    if (fallbackProvider.default_model) {
      onChange({
        provider: fallbackProvider.name,
        model: fallbackProvider.default_model,
      });
      return;
    }
    if (models.length > 0) {
      onChange({
        provider: fallbackProvider.name,
        model: models[0].id,
      });
    }
  }, [enabled, fallbackProvider, models, value, onChange]);

  const groupedRows = React.useMemo(() => {
    return providers.map((provider) => {
      const queriedModels = modelsByProvider.get(provider.name) ?? [];
      const providerModels =
        queriedModels.length > 0
          ? [...queriedModels]
          : provider.default_model
            ? [{ id: provider.default_model }]
            : [];
      const ids = new Set(providerModels.map((m) => m.id));
      const hideCliDefault =
        isCliProvider(provider) &&
        provider.default_model === "default" &&
        providerModels.length > 0;
      if (
        provider.default_model &&
        !ids.has(provider.default_model) &&
        !hideCliDefault
      ) {
        providerModels.unshift({ id: provider.default_model });
        ids.add(provider.default_model);
      }
      if (
        value?.provider === provider.name &&
        value.model &&
        !(
          isCliProvider(provider) &&
          value.model === "default" &&
          providerModels.length > 0
        ) &&
        !ids.has(value.model)
      ) {
        providerModels.unshift({ id: value.model });
      }
      return { provider, models: providerModels };
    });
  }, [providers, modelsByProvider, value]);

  const currentLabel = value
    ? `${value.provider} · ${value.model}`
    : !enabled
      ? "Select model"
      : providers.length === 0
      ? "No provider configured"
      : "Select a model";

  const handleChange = (raw: string) => {
    const next = parseRowValue(raw);
    onChange(next);
  };

  const isRefreshing =
    providersQuery.isFetching || modelQueries.some((query) => query.isFetching);
  const handleRefresh = (event: React.MouseEvent<HTMLButtonElement>) => {
    event.preventDefault();
    event.stopPropagation();

    void (async () => {
      await providersQuery.refetch();
      await Promise.allSettled(modelQueries.map((query) => query.refetch()));
    })();
  };

  const refreshModelsButton = (
    <Button
      type="button"
      variant="ghost"
      size="icon-xs"
      className="-mr-1 size-6 text-muted-foreground hover:text-foreground"
      onClick={handleRefresh}
      disabled={isRefreshing}
      aria-label="Refresh all AI models"
      title="Refresh all AI models"
    >
      <RefreshCw
        className={cn("h-3.5 w-3.5", isRefreshing && "animate-spin")}
        aria-hidden="true"
      />
    </Button>
  );

  return (
    <ModelSelector
      value={value ? rowValue(value.provider, value.model) : ""}
      onValueChange={handleChange}
    >
      <ModelSelectorTrigger>
        <ModelSelectorValue>{currentLabel}</ModelSelectorValue>
      </ModelSelectorTrigger>
      <ModelSelectorContent>
        <ModelSelectorGroup>
          <ModelSelectorLabel trailing={refreshModelsButton}>
            Models
          </ModelSelectorLabel>
        </ModelSelectorGroup>
        {groupedRows.length === 0 ? (
          <ModelSelectorGroup>
            <ModelSelectorEmpty className="block px-2 py-1.5 text-xs">
              No AI providers configured
            </ModelSelectorEmpty>
          </ModelSelectorGroup>
        ) : (
          groupedRows.map(({ provider, models: rows }) => (
            <ModelSelectorGroup key={provider.name}>
              <ModelSelectorLabel
                provider={provider.name}
                kind={provider.kind}
              />
              {rows.length === 0 ? (
                <ModelSelectorItem
                  value={rowValue(provider.name, "__placeholder__")}
                  disabled
                >
                  <ModelSelectorEmpty>No models found</ModelSelectorEmpty>
                </ModelSelectorItem>
              ) : (
                rows.map((model) => (
                  <ModelSelectorItem
                    key={`${provider.name}-${model.id}`}
                    value={rowValue(provider.name, model.id)}
                  >
                    <ModelSelectorName>{model.id}</ModelSelectorName>
                  </ModelSelectorItem>
                ))
              )}
            </ModelSelectorGroup>
          ))
        )}
      </ModelSelectorContent>
    </ModelSelector>
  );
}
