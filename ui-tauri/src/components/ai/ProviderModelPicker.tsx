/**
 * Provider/model picker for the AI chat composer, laid out after T3 Code's
 * picker (see ModelPickerContent for the provenance note): a quiet trigger in
 * the composer toolbar, a provider rail with Favorites, ranked search across
 * every provider, keyboard navigation, and an adjacent reasoning-effort menu.
 *
 * Privacy contract (docs/reference/ai.md, docs/reference/privacy-and-security.md):
 * - Mounting, opening, hovering, searching and switching providers never start
 *   model discovery. `ai.list_models` with `refresh` runs only from the
 *   explicit **Check models** action, for that one provider.
 * - Remote and TEE providers need Kassiber's off-device acknowledgement before
 *   the first selection or discovery.
 * - The `local` / `remote` / `tee` posture is shown on every model row and on
 *   the collapsed trigger, so the user can tell whether a prompt is about to
 *   leave the device. The trigger also keeps the provider's name: two providers
 *   can expose the same model id.
 */

import * as React from "react";
import { useQueries } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Brain } from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { AssistantThinkingEffort } from "./assistantSession";
import {
  DaemonRequestError,
  daemonQueryKey,
  useDaemon,
  useDaemonMutation,
} from "@/daemon/client";
import { getTransport, type DaemonEnvelope } from "@/daemon/transport";
import {
  selectedModelReasoningEfforts,
  type AiProviderKind,
  type AiModelsListData,
  type AiProviderRow,
  type AiProvidersListData,
} from "@/lib/aiCapabilities";
import { formatShortcut } from "@/lib/shortcutLabel";
import { useUiStore } from "@/store/ui";
import {
  ComposerControl,
  ComposerControlChevron,
  ComposerControlSeparator,
} from "./ComposerControl";
import {
  ModelPickerContent,
  type ModelPickerDiscoveryState,
} from "./ModelPickerContent";
import { MODEL_PICKER_SHORTCUT } from "./modelPickerKeys";
import { PostureBadge, ProviderGlyph } from "./ProviderGlyph";
import { isCliProvider, providerDisplayName } from "./providerIdentity";
import {
  dedupeProviderRows,
  filterModelsByPrivacy,
  modelPrivacyPosture,
} from "./providerModelSearch";

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
  /** Controlled open state, e.g. for the composer's Mod+Shift+M shortcut. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
}

// Levels we can label/type. When a model advertises a specific subset we show
// only those; otherwise we offer all of them. "auto" is the default and means
// "don't override the model" — it is always offered first.
const KNOWN_EFFORTS: AssistantThinkingEffort[] = [
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
  "ultra",
];

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
  open: controlledOpen,
  onOpenChange,
}: ProviderModelPickerProps) {
  const { t } = useTranslation("assistant");
  const dataMode = useUiStore((state) => state.dataMode);
  const daemonSession = useUiStore((state) => state.daemonSession);
  const favorites = useUiStore((state) => state.assistantModelFavorites);
  const toggleFavorite = useUiStore(
    (state) => state.toggleAssistantModelFavorite,
  );
  const [uncontrolledOpen, setUncontrolledOpen] = React.useState(false);
  const open = controlledOpen ?? uncontrolledOpen;
  const [thinkingOpen, setThinkingOpen] = React.useState(false);
  const [localOnly, setLocalOnly] = React.useState(false);
  const searchInputRef = React.useRef<HTMLInputElement>(null);
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
  // Resolve the selection from stored configuration. Discovery remains disabled
  // until the user checks this provider, including loopback and native CLIs.
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
      next.set(provider.name, []);
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

  const selectedModelRow = value
    ? models.find((model) => model.id === value.model)
    : undefined;
  const currentProvider = value
    ? providers.find((p) => p.name === value.provider)
    : undefined;
  const currentProviderLabel = value
    ? currentProvider
      ? providerDisplayName(currentProvider)
      : value.provider
    : null;
  // Keep the provider name in the collapsed label ("Ollama · qwen3.6:35b"):
  // two providers can expose the same model id, and the user must be able to
  // tell which endpoint/account a prompt is about to go to.
  const triggerLabel = value
    ? `${currentProviderLabel} · ${selectedModelRow?.display_name || value.model}`
    : !enabled
      ? t("modelPicker.selectModel")
      : providers.length === 0
        ? t("modelPicker.noProviderConfigured")
        : t("modelPicker.selectAModel");
  // Posture of what the next prompt would reach: a model-level posture (an
  // OpenCode source proven local) when discovery supplied one, otherwise the
  // provider's configured kind.
  const triggerPosture: AiProviderKind | null = currentProvider
    ? selectedModelRow
      ? modelPrivacyPosture(currentProvider, selectedModelRow)
      : currentProvider.kind
    : null;

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

  const discoveryByProvider = React.useMemo(() => {
    const next = new Map<string, ModelPickerDiscoveryState>();
    providers.forEach((provider, index) => {
      const query = modelQueries[index];
      next.set(provider.name, {
        isFetching: query?.isFetching === true,
        error: query?.error instanceof Error ? query.error : null,
        snapshot: modelSnapshotsByProvider.get(provider.name),
      });
    });
    return next;
  }, [modelQueries, modelSnapshotsByProvider, providers]);

  const setPickerOpen = (next: boolean) => {
    onOpenChange?.(next);
    if (controlledOpen === undefined) setUncontrolledOpen(next);
    onOverlayOpenChange?.(next || thinkingOpen);
    if (!next) acknowledgeProvider.reset();
  };

  const ensureAcknowledged = async (
    provider: AiProviderRow,
    messageKey:
      | "modelPicker.remoteConfirm"
      | "modelPicker.remoteDiscoveryConfirm",
  ): Promise<boolean> => {
    if (provider.kind === "local" || provider.acknowledged_at) return true;
    if (
      !window.confirm(t(messageKey, { provider: providerDisplayName(provider) }))
    ) {
      return false;
    }
    await acknowledgeProvider.mutateAsync({ name: provider.name });
    await providersQuery.refetch();
    return true;
  };

  const selectModel = async (provider: AiProviderRow, model: string) => {
    if (!(await ensureAcknowledged(provider, "modelPicker.remoteConfirm"))) {
      return;
    }
    onChange({ provider: provider.name, model });
    setPickerOpen(false);
  };

  const checkModels = async (provider: AiProviderRow) => {
    const query =
      modelQueries[providers.findIndex((row) => row.name === provider.name)];
    if (!query) return;
    if (
      !(await ensureAcknowledged(provider, "modelPicker.remoteDiscoveryConfirm"))
    ) {
      return;
    }
    // Native model discovery already probes this provider. Global runtime
    // status would additionally start every other CLI without authorization.
    await query.refetch();
  };

  React.useEffect(
    () => () => onOverlayOpenChange?.(false),
    [onOverlayOpenChange],
  );

  const pickerShortcut = formatShortcut(MODEL_PICKER_SHORTCUT);
  const postureHint = triggerPosture
    ? t(`modelPicker.postureHint.${triggerPosture}`)
    : null;
  const ackError = acknowledgeProvider.error
    ? acknowledgeProvider.error instanceof Error
      ? acknowledgeProvider.error.message
      : String(acknowledgeProvider.error)
    : null;

  return (
    <TooltipProvider delayDuration={300}>
      <Popover open={open} onOpenChange={setPickerOpen}>
        <Tooltip>
          <TooltipTrigger asChild>
            <PopoverTrigger asChild disabled={!enabled}>
              <ComposerControl
                aria-label={`${t("modelPicker.models")}: ${triggerLabel}${
                  postureHint ? ` (${postureHint})` : ""
                }`}
                data-chat-provider-model-picker
                className="min-w-0 max-w-full shrink justify-start"
              >
                {currentProvider ? (
                  <ProviderGlyph provider={currentProvider} className="size-4" />
                ) : null}
                <span className="min-w-0 truncate">{triggerLabel}</span>
                {triggerPosture ? <PostureBadge posture={triggerPosture} /> : null}
                <ComposerControlChevron />
              </ComposerControl>
            </PopoverTrigger>
          </TooltipTrigger>
          <TooltipContent side="top" sideOffset={6}>
            <span className="flex items-center gap-2">
              <span>
                {triggerLabel}
                {postureHint ? ` · ${postureHint}` : ""}
              </span>
              <span className="opacity-60">{pickerShortcut}</span>
            </span>
          </TooltipContent>
        </Tooltip>
        <PopoverContent
          align="start"
          side="top"
          sideOffset={8}
          className="w-[min(25rem,calc(100vw-2rem))] overflow-hidden rounded-xl p-0"
          onOpenAutoFocus={(event) => {
            // Focus the search field, as T3 Code does, so typing filters and
            // the arrow keys drive the list straight away.
            event.preventDefault();
            searchInputRef.current?.focus({ preventScroll: true });
          }}
        >
          <ModelPickerContent
            groups={visibleGroups}
            value={value}
            favorites={favorites}
            localOnly={localOnly}
            onLocalOnlyChange={setLocalOnly}
            discoveryByProvider={discoveryByProvider}
            checkDisabled={acknowledgeProvider.isPending}
            selectDisabled={acknowledgeProvider.isPending}
            footerError={ackError}
            searchInputRef={searchInputRef}
            onSelect={(provider, model) => void selectModel(provider, model)}
            onToggleFavorite={(provider, model) =>
              toggleFavorite({ provider, model })
            }
            onCheckModels={(provider) => void checkModels(provider)}
          />
        </PopoverContent>
      </Popover>

      {showThinkingEffort && onThinkingEffortChange ? (
        <>
          <ComposerControlSeparator />
          <DropdownMenu
            open={thinkingOpen}
            onOpenChange={(next) => {
              setThinkingOpen(next);
              onOverlayOpenChange?.(open || next);
            }}
          >
            <DropdownMenuTrigger asChild disabled={!enabled}>
              <ComposerControl
                aria-label={`${t("composer.reasoningEffort")}: ${t(
                  `composer.effort.${thinkingEffort}`,
                )}`}
                title={t("composer.reasoningEffort")}
              >
                <Brain className="size-4" aria-hidden="true" />
                <span>{t(`composer.effort.${thinkingEffort}`)}</span>
                <ComposerControlChevron />
              </ComposerControl>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" side="top" className="min-w-44">
              <DropdownMenuLabel className="text-xs font-medium text-muted-foreground">
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
                      <span className="flex min-w-0 flex-col">
                        <span className="flex items-center gap-1.5">
                          {t(`composer.effort.${effort}`)}
                          {effort === "auto" ? (
                            <span className="rounded border border-border px-1 text-3xs leading-4 text-muted-foreground">
                              {t("composer.effortDefault")}
                            </span>
                          ) : null}
                        </span>
                        {effort === "auto" ? (
                          <span className="text-2xs text-muted-foreground">
                            {t("composer.effortAutoHint")}
                          </span>
                        ) : null}
                      </span>
                    </DropdownMenuRadioItem>
                  ),
                )}
              </DropdownMenuRadioGroup>
            </DropdownMenuContent>
          </DropdownMenu>
        </>
      ) : null}
    </TooltipProvider>
  );
}
