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
  Search,
  ShieldCheck,
  type LucideIcon,
} from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import type { AssistantThinkingEffort } from "./assistantSession";
import {
  DaemonRequestError,
  daemonQueryKey,
  useDaemon,
  useDaemonMutation,
} from "@/daemon/client";
import { getTransport, type DaemonEnvelope } from "@/daemon/transport";
import {
  isNativeAiProviderLocator,
  nativeAiProviderRuntime,
  selectedModelReasoningEfforts,
  type AiProviderKind,
  type AiModelsListData,
  type AiProviderRow,
  type AiProviderRuntimeStatusData,
  type AiProvidersListData,
} from "@/lib/aiCapabilities";
import { useUiStore } from "@/store/ui";
import { cn } from "@/lib/utils";
import {
  dedupeProviderRows,
  filterModelsByPrivacy,
  filterModelRows,
  sortModelRowsByPosture,
  modelPrivacyPosture,
  providerRuntimeSelectable,
  providerRuntimeTone,
} from "./providerModelSearch";
import { PROVIDER_BRAND_ICON_BY_RUNTIME } from "./providerBrandIcons";

interface ProviderModelPickerProps {
  value: { provider: string; model: string } | null;
  onChange: (next: { provider: string; model: string } | null) => void;
  onOverlayOpenChange?: (open: boolean) => void;
  enabled?: boolean;
  onActiveProviderKindChange?: (kind: AiProviderKind | null) => void;
  /** When supported, render a separate reasoning-effort menu beside the picker. */
  thinkingEffort?: AssistantThinkingEffort;
  onThinkingEffortChange?: (effort: AssistantThinkingEffort) => void;
  showThinkingEffort?: boolean;
}

// Levels we can label/type. When a model advertises a specific subset we show
// only those; otherwise we offer all of them. "auto" is the default and means
// "don't override the model" — it is intentionally not a selectable row, so
// until the user picks a level nothing is checked.
const KNOWN_EFFORTS: AssistantThinkingEffort[] = [
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
];

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

function isCliProvider(provider: AiProviderRow): boolean {
  return isNativeAiProviderLocator(provider.base_url);
}

function providerDisplayName(provider: AiProviderRow): string {
  return provider.display_name?.trim() || provider.name;
}

function runtimeProviderName(
  provider: AiProviderRow,
): "codex" | "claude" | "opencode" | null {
  return nativeAiProviderRuntime(provider.base_url);
}

async function fetchProviderModels(
  provider: string,
): Promise<DaemonEnvelope<AiModelsListData>> {
  const envelope = await getTransport().invoke<AiModelsListData>({
    kind: "ai.list_models",
    args: { provider, refresh: true },
  });
  if (envelope.kind === "error" || envelope.error) {
    throw new DaemonRequestError("ai.list_models", envelope);
  }
  return envelope;
}

export function ProviderModelPicker({
  value,
  onChange,
  onOverlayOpenChange,
  enabled = true,
  onActiveProviderKindChange,
  thinkingEffort = "auto",
  onThinkingEffortChange,
  showThinkingEffort = false,
}: ProviderModelPickerProps) {
  const { t } = useTranslation("assistant");
  const dataMode = useUiStore((state) => state.dataMode);
  const daemonSession = useUiStore((state) => state.daemonSession);
  const [open, setOpen] = React.useState(false);
  const [thinkingOpen, setThinkingOpen] = React.useState(false);
  const [activeProviderName, setActiveProviderName] = React.useState<
    string | null
  >(null);
  const [search, setSearch] = React.useState("");
  const [localOnly, setLocalOnly] = React.useState(false);
  const acknowledgeProvider = useDaemonMutation("ai.providers.acknowledge");
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
  const providers = React.useMemo<AiProviderRow[]>(() => {
    if (
      providersQuery.data?.kind !== "ai.providers.list" ||
      !providersQuery.data.data
    ) {
      return [];
    }
    return dedupeProviderRows(
      providersQuery.data.data.providers,
      value?.provider ?? providersQuery.data.data.default,
    );
  }, [providersQuery.data, value?.provider]);
  const runtimeQuery = useDaemon<AiProviderRuntimeStatusData>(
    "ai.provider_runtime.status",
    { refresh: true },
    {
      enabled: false,
      retry: false,
      staleTime: Infinity,
      gcTime: 30 * 60 * 1000,
    },
  );
  const runtimeByProvider = React.useMemo(() => {
    const rows =
      runtimeQuery.data?.kind === "ai.provider_runtime.status" &&
      runtimeQuery.data.data
        ? runtimeQuery.data.data.providers
        : [];
    return new Map(rows.map((row) => [row.provider, row]));
  }, [runtimeQuery.data]);

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
      queryFn: () => fetchProviderModels(provider.name),
      enabled: false,
      refetchOnMount: false,
      staleTime: 5 * 60 * 1000,
      gcTime: 60 * 60 * 1000,
      meta: { shellProgress: false },
    })),
  });
  const modelSnapshotsByProvider = React.useMemo(() => {
    const next = new Map<string, AiModelsListData>();
    providers.forEach((provider, index) => {
      const result = modelQueries[index]?.data;
      if (result?.kind === "ai.list_models" && result.data) {
        next.set(provider.name, result.data);
      }
    });
    return next;
  }, [modelQueries, providers]);
  const modelsByProvider = React.useMemo(() => {
    const next = new Map<string, AiModelsListData["models"]>();
    providers.forEach((provider, index) => {
      const result = modelQueries[index];
      if (result?.data?.kind === "ai.list_models" && result.data.data) {
        next.set(provider.name, result.data.data.models);
        return;
      }
      const runtime = runtimeProviderName(provider);
      if (runtime) {
        next.set(provider.name, runtimeByProvider.get(runtime)?.models ?? []);
        return;
      }
      next.set(provider.name, []);
    });
    return next;
  }, [providers, modelQueries, runtimeByProvider]);
  const models = React.useMemo(
    () =>
      selectedProvider
        ? (modelsByProvider.get(selectedProvider.name) ?? [])
        : [],
    [selectedProvider, modelsByProvider],
  );

  React.useEffect(() => {
    if (!open) return;
    const requested = value?.provider ?? fallbackProvider?.name ?? null;
    if (
      activeProviderName === null ||
      !providers.some((provider) => provider.name === activeProviderName)
    ) {
      setActiveProviderName(requested);
    }
  }, [activeProviderName, fallbackProvider?.name, open, providers, value?.provider]);

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
  }, [enabled, fallbackProvider, models, value, onChange, selectedProvider]);

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
  const visibleGroups = React.useMemo(() => {
    if (!localOnly) return groupedRows;
    return groupedRows
      .map(({ provider, models: providerModels }) => ({
        provider,
        models: filterModelsByPrivacy(provider, providerModels, true),
      }))
      .filter(
        ({ provider, models: providerModels }) =>
          provider.kind === "local" || providerModels.length > 0,
      );
  }, [groupedRows, localOnly]);

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

  // Keep the provider name in the collapsed label ("Ollama · qwen3.6:35b"):
  // two providers can expose the same model id, and the user must be able to
  // tell which endpoint/account a prompt is about to go to.
  const triggerLabel = currentLabel;
  const selectedRuntimeName = selectedProvider
    ? runtimeProviderName(selectedProvider)
    : null;
  const TriggerKindIcon =
    (selectedRuntimeName
      ? PROVIDER_BRAND_ICON_BY_RUNTIME[selectedRuntimeName]
      : undefined) ?? KIND_ICON[selectedProvider?.kind ?? "local"];

  const activeGroup =
    visibleGroups.find(({ provider }) => provider.name === activeProviderName) ??
    visibleGroups[0] ??
    null;
  const activeRuntimeName = activeGroup
    ? runtimeProviderName(activeGroup.provider)
    : null;
  const activeRuntime = activeRuntimeName
    ? runtimeByProvider.get(activeRuntimeName)
    : null;
  const runtimeSnapshot =
    runtimeQuery.data?.kind === "ai.provider_runtime.status"
      ? runtimeQuery.data.data
      : undefined;
  const activeDiscovery = activeRuntimeName
    ? activeGroup
      ? modelSnapshotsByProvider.get(activeGroup.provider.name) ?? runtimeSnapshot
      : runtimeSnapshot
    : activeGroup
      ? modelSnapshotsByProvider.get(activeGroup.provider.name)
      : undefined;
  const activeModelQuery = activeGroup
    ? modelQueries[
        providers.findIndex((provider) => provider.name === activeGroup.provider.name)
      ]
    : undefined;
  const filteredModels = activeGroup
    ? sortModelRowsByPosture(
        activeGroup.provider,
        filterModelRows(activeGroup.models, search),
      )
    : [];

  const selectModel = async (provider: AiProviderRow, model: string) => {
    if (provider.kind !== "local" && !provider.acknowledged_at) {
      if (
        !window.confirm(
          t("modelPicker.remoteConfirm", {
            provider: providerDisplayName(provider),
          }),
        )
      ) {
        return;
      }
      await acknowledgeProvider.mutateAsync({ name: provider.name });
      await providersQuery.refetch();
    }
    onChange({ provider: provider.name, model });
    setOpen(false);
    onOverlayOpenChange?.(thinkingOpen);
  };

  const checkActiveModels = async () => {
    if (!activeGroup || !activeModelQuery) return;
    const provider = activeGroup.provider;
    if (provider.kind !== "local" && !provider.acknowledged_at) {
      if (
        !window.confirm(
          t("modelPicker.remoteDiscoveryConfirm", {
            provider: providerDisplayName(provider),
          }),
        )
      ) {
        return;
      }
      await acknowledgeProvider.mutateAsync({ name: provider.name });
      await providersQuery.refetch();
    }
    await activeModelQuery.refetch();
  };

  React.useEffect(
    () => () => onOverlayOpenChange?.(false),
    [onOverlayOpenChange],
  );

  return (
    <>
      <Popover
        open={open}
        onOpenChange={(next) => {
          setOpen(next);
          onOverlayOpenChange?.(next || thinkingOpen);
          if (!next) {
            setSearch("");
            acknowledgeProvider.reset();
          }
        }}
      >
        <PopoverTrigger asChild disabled={!enabled}>
          <button
            type="button"
            className="flex w-fit max-w-full items-center gap-1.5 rounded-full text-sm leading-none text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-60"
            aria-label={t("modelPicker.models")}
          >
            <TriggerKindIcon className="size-4 shrink-0" aria-hidden="true" />
            <span className="truncate">{triggerLabel}</span>
            <ChevronDown
              className="size-3.5 shrink-0 opacity-70"
              aria-hidden="true"
            />
          </button>
        </PopoverTrigger>
        <PopoverContent
          align="start"
          side="top"
          sideOffset={8}
          className="w-[min(34rem,calc(100vw-2rem))] overflow-hidden p-0"
        >
          <div className="flex h-[min(27rem,70vh)] min-h-72">
            <div className="flex w-14 shrink-0 flex-col gap-1 overflow-y-auto border-r border-border/60 bg-muted/40 p-1">
              {visibleGroups.map(({ provider }) => {
                const active = activeGroup?.provider.name === provider.name;
                const runtimeName = runtimeProviderName(provider);
                const Icon =
                  (runtimeName
                    ? PROVIDER_BRAND_ICON_BY_RUNTIME[runtimeName]
                    : undefined) ?? KIND_ICON[provider.kind];
                const runtime = runtimeName
                  ? runtimeByProvider.get(runtimeName)
                  : null;
                const discoveryStale = runtimeName
                  ? runtimeSnapshot?.stale
                  : modelSnapshotsByProvider.get(provider.name)?.stale;
                const runtimeTone = discoveryStale
                  ? "attention"
                  : providerRuntimeTone(runtime);
                return (
                  <button
                    key={provider.name}
                    type="button"
                    onClick={() => {
                      setActiveProviderName(provider.name);
                      setSearch("");
                    }}
                    className={cn(
                      "relative flex aspect-square w-full items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
                      active && "bg-background text-foreground shadow-sm",
                    )}
                    aria-label={`${providerDisplayName(provider)}${
                      runtime ? ` · ${runtime.message}` : ""
                    }`}
                    title={`${providerDisplayName(provider)} · ${
                      runtime?.message ?? KIND_BADGE_LABEL[provider.kind]
                    }`}
                  >
                    <Icon className="size-5" aria-hidden="true" />
                    <span
                      className={cn(
                        "absolute right-1 top-1 size-1.5 rounded-full",
                        runtimeTone === "ready" && "bg-emerald-500",
                        runtimeTone === "attention" && "bg-amber-500",
                        runtimeTone === "unavailable" && "bg-destructive",
                        !runtime &&
                          provider.kind === "local" &&
                          "bg-emerald-500",
                        !runtime &&
                          provider.kind === "remote" &&
                          "bg-amber-500",
                        !runtime && provider.kind === "tee" && "bg-sky-500",
                      )}
                      aria-hidden="true"
                    />
                  </button>
                );
              })}
            </div>

            <div className="flex min-w-0 flex-1 flex-col">
              <div className="border-b border-border/60 p-3">
                <div className="mb-2 flex items-center justify-between gap-2">
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">
                      {activeGroup
                        ? providerDisplayName(activeGroup.provider)
                        : t("modelPicker.models")}
                    </p>
                    {activeGroup ? (
                      <p className="truncate text-xs text-muted-foreground">
                        {isCliProvider(activeGroup.provider)
                          ? activeRuntime?.message ??
                            (runtimeQuery.isFetching
                              ? t("modelPicker.checkingProvider")
                              : t("modelPicker.cliChatOnly"))
                          : activeGroup.provider.base_url}
                      </p>
                    ) : null}
                    {activeDiscovery?.stale ? (
                      <p className="truncate text-xs text-amber-600 dark:text-amber-400">
                        {t("modelPicker.staleModels", {
                          error: activeDiscovery.error?.message ?? "",
                        })}
                      </p>
                    ) : null}
                  </div>
                  <div className="flex shrink-0 items-center gap-2">
                    {activeGroup ? (
                      <Button
                        type="button"
                        size="sm"
                        variant="outline"
                        data-model-picker-check
                        disabled={
                          acknowledgeProvider.isPending ||
                          activeModelQuery?.isFetching
                        }
                        onClick={() => void checkActiveModels()}
                      >
                        <RefreshCw
                          className={cn(
                            "size-3.5",
                            activeModelQuery?.isFetching && "animate-spin",
                          )}
                          aria-hidden="true"
                        />
                        {activeModelQuery?.isFetching
                          ? t("modelPicker.checkingModels")
                          : t("modelPicker.checkModels")}
                      </Button>
                    ) : null}
                    <div
                      className="inline-flex rounded-md bg-muted p-0.5 text-2xs"
                      role="group"
                      aria-label={t("modelPicker.privacyFilter")}
                    >
                      <button
                        type="button"
                        aria-pressed={!localOnly}
                        onClick={() => setLocalOnly(false)}
                        className={cn(
                          "rounded px-1.5 py-1 text-muted-foreground",
                          !localOnly && "bg-background text-foreground shadow-sm",
                        )}
                      >
                        {t("modelPicker.allModels")}
                      </button>
                      <button
                        type="button"
                        aria-pressed={localOnly}
                        onClick={() => setLocalOnly(true)}
                        className={cn(
                          "rounded px-1.5 py-1 text-muted-foreground",
                          localOnly && "bg-background text-foreground shadow-sm",
                        )}
                      >
                        {t("modelPicker.localModels")}
                      </button>
                    </div>
                  </div>
                </div>
                <div className="relative">
                  <Search
                    className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground"
                    aria-hidden="true"
                  />
                  <Input
                    value={search}
                    onChange={(event) => setSearch(event.target.value)}
                    placeholder={t("modelPicker.searchModels")}
                    className="h-8 pl-8 text-xs"
                  />
                </div>
              </div>

              <div
                className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-1.5"
                data-model-picker-content
              >
                {!activeGroup ? (
                  <p className="p-3 text-sm text-muted-foreground">
                    {t("modelPicker.noProviders")}
                  </p>
                ) : filteredModels.length === 0 ? (
                  <p className="p-3 text-sm text-muted-foreground">
                    {activeRuntime && activeRuntime.state !== "ready"
                      ? activeRuntime.message
                      : activeModelQuery?.isFetching
                        ? t("modelPicker.checkingModels")
                        : activeModelQuery?.error instanceof Error
                          ? activeModelQuery.error.message
                          : search
                            ? t("modelPicker.noMatchingModels")
                            : t("modelPicker.noModels")}
                  </p>
                ) : (
                  filteredModels.map((model) => {
                    const selected =
                      value?.provider === activeGroup.provider.name &&
                      value.model === model.id;
                    const privacyPosture = modelPrivacyPosture(
                      activeGroup.provider,
                      model,
                    );
                    return (
                      <button
                        key={model.id}
                        type="button"
                        disabled={
                          acknowledgeProvider.isPending ||
                          Boolean(!providerRuntimeSelectable(activeRuntime))
                        }
                        onClick={() =>
                          void selectModel(activeGroup.provider, model.id)
                        }
                        className={cn(
                          "flex w-full items-center gap-3 rounded-md px-3 py-2 text-left outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-45",
                          selected && "bg-muted/70",
                        )}
                      >
                        <span className="min-w-0 flex-1">
                          <span className="block truncate text-sm font-medium">
                            {model.display_name || model.id}
                          </span>
                          {model.display_name ? (
                            <span className="block truncate font-mono text-2xs text-muted-foreground">
                              {model.id}
                            </span>
                          ) : null}
                        </span>
                        <span
                          className={cn(
                            "shrink-0 rounded-full px-1.5 py-0.5 text-3xs font-medium uppercase",
                            KIND_TONE[privacyPosture],
                          )}
                          title={model.privacy_reason}
                        >
                          {KIND_BADGE_LABEL[privacyPosture]}
                        </span>
                        {selected ? (
                          <Check
                            className="size-4 shrink-0"
                            aria-hidden="true"
                          />
                        ) : null}
                      </button>
                    );
                  })
                )}
              </div>

              {acknowledgeProvider.error ? (
                <div className="border-t border-border/60 p-2">
                  <p className="px-2 text-xs text-destructive">
                    {acknowledgeProvider.error instanceof Error
                      ? acknowledgeProvider.error.message
                      : String(acknowledgeProvider.error)}
                  </p>
                </div>
              ) : null}
            </div>
          </div>
        </PopoverContent>
      </Popover>

      {showThinkingEffort && onThinkingEffortChange ? (
        <>
          <span
            className="mx-1 hidden h-4 w-px shrink-0 bg-border sm:block"
            aria-hidden="true"
          />
          <DropdownMenu
            open={thinkingOpen}
            onOpenChange={(next) => {
              setThinkingOpen(next);
              onOverlayOpenChange?.(open || next);
            }}
          >
            <DropdownMenuTrigger asChild disabled={!enabled}>
              <button
                type="button"
                className="flex shrink-0 items-center gap-1 rounded-full text-sm leading-none text-muted-foreground transition-colors hover:text-foreground focus-visible:outline-none disabled:cursor-not-allowed disabled:opacity-60"
                aria-label={t("composer.reasoningEffort")}
              >
                <span>{t(`composer.effort.${thinkingEffort}`)}</span>
                <ChevronDown
                  className="size-3.5 shrink-0 opacity-70"
                  aria-hidden="true"
                />
              </button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" side="top">
              <DropdownMenuLabel>
                {t("composer.reasoningEffort")}
              </DropdownMenuLabel>
              <DropdownMenuRadioGroup
                value={thinkingEffort}
                onValueChange={(effort) =>
                  onThinkingEffortChange(effort as AssistantThinkingEffort)
                }
              >
                {(["auto", ...effortOptions] as AssistantThinkingEffort[]).map(
                  (effort) => (
                    <DropdownMenuRadioItem key={effort} value={effort}>
                      {t(`composer.effort.${effort}`)}
                    </DropdownMenuRadioItem>
                  ),
                )}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </>
      ) : null}
    </>
  );
}
