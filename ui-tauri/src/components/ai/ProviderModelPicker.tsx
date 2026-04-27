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
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useDaemon } from "@/daemon/client";
import { cn } from "@/lib/utils";

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
}

const KIND_LABELS: Record<ProviderRow["kind"], { label: string; tone: string }> = {
  local: { label: "local", tone: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300" },
  remote: { label: "remote", tone: "bg-amber-500/15 text-amber-700 dark:text-amber-300" },
  tee: { label: "TEE", tone: "bg-sky-500/15 text-sky-700 dark:text-sky-300" },
};

function rowValue(provider: string, model: string): string {
  return `${provider}::${model}`;
}

function parseRowValue(value: string): { provider: string; model: string } | null {
  const idx = value.indexOf("::");
  if (idx < 0) return null;
  return { provider: value.slice(0, idx), model: value.slice(idx + 2) };
}

export function ProviderModelPicker({ value, onChange }: ProviderModelPickerProps) {
  const providersQuery = useDaemon<ProvidersListData>("ai.providers.list");
  const providers = React.useMemo<ProviderRow[]>(
    () =>
      providersQuery.data?.kind === "ai.providers.list" &&
      providersQuery.data.data
        ? providersQuery.data.data.providers
        : [],
    [providersQuery.data],
  );

  // Once the providers list lands, fall back to the default provider's
  // default_model if the parent hasn't picked anything yet.
  React.useEffect(() => {
    if (value || providers.length === 0) return;
    const defaultProvider =
      providers.find((p) => p.is_default) ?? providers[0];
    if (!defaultProvider) return;
    if (defaultProvider.default_model) {
      onChange({
        provider: defaultProvider.name,
        model: defaultProvider.default_model,
      });
    }
  }, [providers, value, onChange]);

  const selectedProvider = value
    ? providers.find((p) => p.name === value.provider)
    : undefined;

  const modelsQuery = useDaemon<ModelsListData>(
    "ai.list_models",
    selectedProvider ? { provider: selectedProvider.name } : undefined,
    {
      enabled: Boolean(selectedProvider),
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
    : providers.length === 0
      ? "No provider configured"
      : "Select a model";

  const handleChange = (raw: string) => {
    const next = parseRowValue(raw);
    onChange(next);
  };

  return (
    <Select
      value={value ? rowValue(value.provider, value.model) : ""}
      onValueChange={handleChange}
    >
      <SelectTrigger className="w-fit border-none bg-transparent! p-0 text-sm text-muted-foreground hover:text-foreground focus:ring-0 shadow-none">
        <SelectValue>
          <span className="truncate">{currentLabel}</span>
        </SelectValue>
      </SelectTrigger>
      <SelectContent
        position="popper"
        side="top"
        align="start"
        className="min-w-72"
      >
        {groupedRows.length === 0 ? (
          <SelectGroup>
            <SelectLabel>No AI providers configured</SelectLabel>
          </SelectGroup>
        ) : (
          groupedRows.map(({ provider, models: rows }) => (
            <SelectGroup key={provider.name}>
              <SelectLabel className="flex items-center gap-2">
                <span>{provider.name}</span>
                <span
                  className={cn(
                    "rounded-full px-1.5 py-0.5 text-[10px] font-medium uppercase",
                    KIND_LABELS[provider.kind].tone,
                  )}
                >
                  {KIND_LABELS[provider.kind].label}
                </span>
              </SelectLabel>
              {rows.length === 0 ? (
                <SelectItem value={rowValue(provider.name, "__placeholder__")} disabled>
                  <span className="text-muted-foreground">
                    No models found · open Settings to set a default
                  </span>
                </SelectItem>
              ) : (
                rows.map((model) => (
                  <SelectItem
                    key={`${provider.name}-${model.id}`}
                    value={rowValue(provider.name, model.id)}
                  >
                    <span className="font-mono text-xs">{model.id}</span>
                  </SelectItem>
                ))
              )}
            </SelectGroup>
          ))
        )}
      </SelectContent>
    </Select>
  );
}
