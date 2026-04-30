/**
 * Two-level provider/model picker for the AI chat input.
 *
 * Falls back to free-text "type a model name" when the provider's
 * `/v1/models` endpoint returns nothing. A small badge per row makes the
 * `local` / `remote` / `tee` distinction visible — the privacy posture in
 * docs/reference/ai.md depends on the user being able to tell at a glance
 * whether a prompt is about to leave the device.
 */

import * as React from "react";

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
import { useDaemon } from "@/daemon/client";

export interface ProviderRow {
  name: string;
  base_url: string;
  kind: "local" | "remote" | "tee";
  default_model?: string | null;
  notes?: string | null;
  acknowledged_at?: string | null;
  has_api_key: boolean;
  is_default: boolean;
}

interface ProvidersListData {
  providers: ProviderRow[];
  default: string | null;
}

interface ModelsListData {
  provider: string;
  models: { id: string; owned_by?: string }[];
}

interface ProviderModelPickerProps {
  value: { provider: string; model: string } | null;
  onChange: (next: { provider: string; model: string } | null) => void;
  enabled?: boolean;
}

function rowValue(provider: string, model: string): string {
  return `${provider}::${model}`;
}

function parseRowValue(value: string): { provider: string; model: string } | null {
  const idx = value.indexOf("::");
  if (idx < 0) return null;
  return { provider: value.slice(0, idx), model: value.slice(idx + 2) };
}

export function ProviderModelPicker({
  value,
  onChange,
  enabled = true,
}: ProviderModelPickerProps) {
  const providersQuery = useDaemon<ProvidersListData>(
    "ai.providers.list",
    undefined,
    { enabled },
  );
  const providers = React.useMemo<ProviderRow[]>(
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

  const modelsQuery = useDaemon<ModelsListData>(
    "ai.list_models",
    selectedProvider ? { provider: selectedProvider.name } : undefined,
    {
      enabled: enabled && Boolean(selectedProvider),
      // Models lists are stable for the session; don't poll.
      staleTime: 5 * 60 * 1000,
    },
  );
  const models = React.useMemo(
    () =>
      modelsQuery.data?.kind === "ai.list_models" && modelsQuery.data.data
        ? modelsQuery.data.data.models
        : [],
    [modelsQuery.data],
  );

  // Once providers (and, if needed, models) land, seed a selection so the
  // user can send a chat without first opening Settings. Prefer the saved
  // `default_model`; otherwise pick the first model the provider advertises.
  React.useEffect(() => {
    if (!enabled) return;
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
      const providerModels =
        provider.name === selectedProvider?.name
          ? models
          : provider.default_model
            ? [{ id: provider.default_model }]
            : [];
      const ids = new Set(providerModels.map((m) => m.id));
      if (provider.default_model && !ids.has(provider.default_model)) {
        providerModels.unshift({ id: provider.default_model });
      }
      return { provider, models: providerModels };
    });
  }, [providers, selectedProvider, models]);

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

  return (
    <ModelSelector
      value={value ? rowValue(value.provider, value.model) : ""}
      onValueChange={handleChange}
    >
      <ModelSelectorTrigger>
        <ModelSelectorValue>{currentLabel}</ModelSelectorValue>
      </ModelSelectorTrigger>
      <ModelSelectorContent>
        {groupedRows.length === 0 ? (
          <ModelSelectorGroup>
            <ModelSelectorLabel>No AI providers configured</ModelSelectorLabel>
          </ModelSelectorGroup>
        ) : (
          groupedRows.map(({ provider, models: rows }) => (
            <ModelSelectorGroup key={provider.name}>
              <ModelSelectorLabel
                provider={provider.name}
                kind={provider.kind}
              />
              {rows.length === 0 ? (
                <ModelSelectorItem value={rowValue(provider.name, "__placeholder__")} disabled>
                  <ModelSelectorEmpty>
                    No models found · open Settings to set a default
                  </ModelSelectorEmpty>
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
