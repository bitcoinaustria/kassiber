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
import { useTranslation } from "react-i18next";
import {
  Check,
  ChevronDown,
  Cloud,
  Cpu,
  RefreshCw,
  ShieldCheck,
  type LucideIcon,
} from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuPortal,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { AssistantThinkingEffort } from "./assistantSession";
import {
  DaemonRequestError,
  daemonQueryKey,
  useDaemon,
} from "@/daemon/client";
import { getTransport, type DaemonEnvelope } from "@/daemon/transport";
import {
  selectedModelReasoningEfforts,
  type AiProviderKind,
  type AiModelsListData,
  type AiProviderRow,
  type AiProvidersListData,
} from "@/lib/aiCapabilities";
import { useUiStore, type DataMode } from "@/store/ui";
import { cn } from "@/lib/utils";

interface ProviderModelPickerProps {
  value: { provider: string; model: string } | null;
  onChange: (next: { provider: string; model: string } | null) => void;
  enabled?: boolean;
  onActiveProviderKindChange?: (kind: AiProviderKind | null) => void;
  /** When supported, the dropdown also offers reasoning-effort levels. */
  thinkingEffort?: AssistantThinkingEffort;
  onThinkingEffortChange?: (effort: AssistantThinkingEffort) => void;
  showThinkingEffort?: boolean;
}

// Levels we can label/type. When a model advertises a specific subset we show
// only those; otherwise we offer all of them. "auto" is the default and means
// "don't override the model" — it is intentionally not a selectable row, so
// until the user picks a level nothing is checked.
const KNOWN_EFFORTS: AssistantThinkingEffort[] = ["low", "medium", "high"];

const KIND_ICON: Record<AiProviderKind, LucideIcon> = {
  local: Cpu,
  remote: Cloud,
  tee: ShieldCheck,
};

const KIND_TONE: Record<AiProviderKind, string> = {
  local: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
  remote: "bg-amber-500/15 text-amber-700 dark:text-amber-300",
  tee: "bg-sky-500/15 text-sky-700 dark:text-sky-300",
};

const KIND_BADGE_LABEL: Record<AiProviderKind, string> = {
  local: "local",
  remote: "remote",
  tee: "TEE",
};

function ProviderKindBadge({ kind }: { kind: AiProviderKind }) {
  const Icon = KIND_ICON[kind];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded-full px-1.5 py-0.5 text-[10px] font-medium uppercase",
        KIND_TONE[kind],
      )}
    >
      <Icon className="h-3 w-3" aria-hidden="true" />
      {KIND_BADGE_LABEL[kind]}
    </span>
  );
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

function providerDisplayName(provider: AiProviderRow): string {
  return provider.display_name?.trim() || provider.name;
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
  thinkingEffort = "auto",
  onThinkingEffortChange,
  showThinkingEffort = false,
}: ProviderModelPickerProps) {
  const { t } = useTranslation("assistant");
  const dataMode = useUiStore((state) => state.dataMode);
  const daemonSession = useUiStore((state) => state.daemonSession);
  const providersQuery = useDaemon<AiProvidersListData>(
    "ai.providers.list",
    undefined,
    {
      enabled,
      // The provider list is small, stable across the whole session, and
      // load-bearing for the picker UX — keep it in cache for the lifetime
      // of the app so the picker never blanks on remount or re-focus.
      staleTime: 30 * 60 * 1000,
      gcTime: Infinity,
    },
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
      staleTime: isCliProvider(provider) ? 5 * 60 * 1000 : 30 * 60 * 1000,
      gcTime: 60 * 60 * 1000,
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
  const models = React.useMemo(
    () =>
      selectedProvider
        ? (modelsByProvider.get(selectedProvider.name) ?? [])
        : [],
    [selectedProvider, modelsByProvider],
  );

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

  const currentProvider = value
    ? providers.find((p) => p.name === value.provider)
    : null;
  const currentProviderLabel = value
    ? currentProvider
      ? providerDisplayName(currentProvider)
      : value.provider
    : null;
  const currentLabel = value
    ? `${currentProviderLabel} · ${value.model}`
    : !enabled
      ? t("modelPicker.selectModel")
      : providers.length === 0
      ? t("modelPicker.noProviderConfigured")
      : t("modelPicker.selectAModel");

  const handleChange = (raw: string) => {
    const next = parseRowValue(raw);
    onChange(next);
  };

  const isRefreshing =
    providersQuery.isFetching || modelQueries.some((query) => query.isFetching);
  const refreshModels = () => {
    void (async () => {
      await providersQuery.refetch();
      await Promise.allSettled(modelQueries.map((query) => query.refetch()));
    })();
  };

  // Show only the reasoning levels the selected model advertises; fall back to
  // the full set when it advertises none.
  const advertisedEfforts = React.useMemo(
    () => selectedModelReasoningEfforts({ selection: value, providers, models }),
    [value, providers, models],
  );
  const effortOptions = React.useMemo<AssistantThinkingEffort[]>(() => {
    const advertisedKnown = KNOWN_EFFORTS.filter((effort) =>
      advertisedEfforts.includes(effort),
    );
    return advertisedKnown.length > 0 ? advertisedKnown : KNOWN_EFFORTS;
  }, [advertisedEfforts]);

  // If a model switch leaves the current level unsupported, drop back to auto.
  React.useEffect(() => {
    if (!showThinkingEffort || !onThinkingEffortChange) return;
    if (thinkingEffort !== "auto" && !effortOptions.includes(thinkingEffort)) {
      onThinkingEffortChange("auto");
    }
  }, [showThinkingEffort, onThinkingEffortChange, thinkingEffort, effortOptions]);

  const triggerLabel = value?.model ?? currentLabel;
  const TriggerKindIcon = KIND_ICON[selectedProvider?.kind ?? "local"];

  // Grouped provider/model list, shared by the top level (no reasoning
  // support) and the "model" submenu (reasoning support).
  const modelList =
    groupedRows.length === 0 ? (
      <DropdownMenuItem disabled>{t("modelPicker.noProviders")}</DropdownMenuItem>
    ) : (
      groupedRows.map(({ provider, models: rows }) => (
        <React.Fragment key={provider.name}>
          <DropdownMenuLabel className="flex items-center gap-2 text-muted-foreground">
            <span>{providerDisplayName(provider)}</span>
            <ProviderKindBadge kind={provider.kind} />
          </DropdownMenuLabel>
          {rows.length === 0 ? (
            <DropdownMenuItem disabled className="font-mono text-xs">
              {t("modelPicker.noModels")}
            </DropdownMenuItem>
          ) : (
            rows.map((model) => {
              const selected =
                value?.provider === provider.name && value.model === model.id;
              return (
                <DropdownMenuItem
                  key={`${provider.name}-${model.id}`}
                  className="font-mono text-xs"
                  onSelect={() => handleChange(rowValue(provider.name, model.id))}
                >
                  <span className="flex-1 truncate">{model.id}</span>
                  {selected ? (
                    <Check className="size-4" aria-hidden="true" />
                  ) : null}
                </DropdownMenuItem>
              );
            })
          )}
        </React.Fragment>
      ))
    );

  const refreshItem = (
    <DropdownMenuItem
      disabled={isRefreshing}
      onSelect={(event) => {
        event.preventDefault();
        refreshModels();
      }}
    >
      <RefreshCw
        className={cn("size-4", isRefreshing && "animate-spin")}
        aria-hidden="true"
      />
      {t("modelPicker.refreshModels")}
    </DropdownMenuItem>
  );

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild disabled={!enabled}>
        <button
          type="button"
          className="flex w-fit max-w-full items-center gap-1.5 rounded-full text-sm leading-none text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-60"
          aria-label={t("modelPicker.models")}
        >
          <TriggerKindIcon className="size-4 shrink-0" aria-hidden="true" />
          <span className="truncate">{triggerLabel}</span>
          <ChevronDown className="size-3.5 shrink-0 opacity-70" aria-hidden="true" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        side="top"
        sideOffset={8}
        className="min-w-56 max-w-[min(20rem,90vw)]"
      >
        {showThinkingEffort && onThinkingEffortChange ? (
          <>
            <DropdownMenuLabel className="text-muted-foreground">
              {t("composer.thinking")}
            </DropdownMenuLabel>
            {effortOptions.map((effort) => (
              <DropdownMenuItem
                key={effort}
                onSelect={() => onThinkingEffortChange(effort)}
              >
                <span className="flex-1">{t(`composer.effort.${effort}`)}</span>
                {thinkingEffort === effort ? (
                  <Check className="size-4" aria-hidden="true" />
                ) : null}
              </DropdownMenuItem>
            ))}
            <DropdownMenuSeparator />
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <span className="truncate">{triggerLabel}</span>
              </DropdownMenuSubTrigger>
              <DropdownMenuPortal>
                <DropdownMenuSubContent className="max-h-[min(60vh,22rem)] min-w-56 max-w-[min(20rem,90vw)] overflow-y-auto">
                  {modelList}
                  <DropdownMenuSeparator />
                  {refreshItem}
                </DropdownMenuSubContent>
              </DropdownMenuPortal>
            </DropdownMenuSub>
          </>
        ) : (
          <>
            {modelList}
            <DropdownMenuSeparator />
            {refreshItem}
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
