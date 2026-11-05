/**
 * Provider/model picker for the AI chat composer, laid out after T3 Code's
 * picker (see ModelPickerContent for the provenance note): a quiet trigger in
 * the composer toolbar, a provider rail with Favorites, ranked search across
 * every provider and keyboard navigation. Like T3 Code's composer picker it is
 * the one place for the model's traits too: a reasoning column docked beside
 * the list, and a footer with the speed toggle and the permission mode (see
 * ComposerModelTraits). Each section shows only what the selected model
 * supports per its capability data.
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
 * - An HTTP provider's models that cannot run the assistant (no tools, no text
 *   output, or an id that names an embedding/speech/image model) are hidden
 *   until **Show all models**, judged from the cached listing alone. The
 *   selected, default and favorite models always stay listed.
 */

import * as React from "react";
import { useQueries } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Zap } from "lucide-react";

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
import { KNOWN_EFFORTS, type AssistantThinkingEffort } from "./assistantSession";
import type { AiPermissionMode } from "@/daemon/stream";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import {
  providerEnabled,
  selectedModelFastMode,
  selectedModelReasoningEfforts,
  type AiProviderKind,
  type AiModelsListData,
  type AiProviderRow,
  type AiProvidersListData,
} from "@/lib/aiCapabilities";
import { formatShortcut } from "@/lib/shortcutLabel";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";
import { ComposerControl, ComposerControlChevron } from "./ComposerControl";
import { ModelPickerEffortColumn, ModelPickerFooter } from "./ComposerModelTraits";
import {
  ModelPickerContent,
  type ModelPickerDiscoveryState,
} from "./ModelPickerContent";
import {
  MODEL_PICKER_EFFORT_SHORTCUT,
  MODEL_PICKER_SHORTCUT,
  modelPickerModelKey,
  takeOpenToEffort,
} from "./modelPickerKeys";
import { permissionModeOption } from "./permissionMode";
import { PostureBadge, ProviderGlyph } from "./ProviderGlyph";
import { providerModelsQueryOptions, useRestoreProviderModelChecks } from "./providerModelQuery";
import { isCliProvider, providerDisplayName } from "./providerIdentity";
import {
  canonicalModelFavorites,
  dedupedProviderAliases,
  dedupeProviderRows,
  httpModelRunsAssistant,
  modelPrivacyPosture,
  storedModelFavoritesFor,
} from "./providerModelSearch";

interface ProviderModelPickerProps {
  value: { provider: string; model: string } | null;
  onChange: (next: { provider: string; model: string } | null) => void;
  onOverlayOpenChange?: (open: boolean) => void;
  enabled?: boolean;
  onActiveProviderKindChange?: (kind: AiProviderKind | null) => void;
  /**
   * Reasoning effort. When the selected model takes one, its levels show in a
   * column beside the list; omit the handler to show them locked (e.g. while
   * a reply streams).
   */
  thinkingEffort?: AssistantThinkingEffort;
  onThinkingEffortChange?: (effort: AssistantThinkingEffort) => void;
  showThinkingEffort?: boolean;
  /**
   * Fast serving mode. The footer's speed toggle shows only when the selected
   * model offers it; omit the handler to show it locked.
   */
  fastMode?: boolean;
  onFastModeChange?: (on: boolean) => void;
  /** The chat's permission mode, in the footer; omit the handler to leave it out. */
  permissionMode?: AiPermissionMode;
  onPermissionModeChange?: (mode: AiPermissionMode) => void;
  /** Controlled open state, e.g. for the composer's Mod+Shift+M shortcut. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
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
  fastMode = false,
  onFastModeChange,
  permissionMode = "approval",
  onPermissionModeChange,
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
  const [showAllModels, setShowAllModels] = React.useState(false);
  const searchInputRef = React.useRef<HTMLInputElement>(null);
  const triggerRef = React.useRef<HTMLButtonElement>(null);
  const contentRef = React.useRef<HTMLDivElement>(null);
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
  const providersData =
    providersQuery.data?.kind === "ai.providers.list"
      ? providersQuery.data.data
      : undefined;
  const allProviders = React.useMemo<AiProviderRow[]>(
    () => providersData?.providers ?? [],
    [providersData],
  );
  // A provider switched off in Settings is not offered at all; the daemon
  // refuses chat with it too.
  const providers = React.useMemo<AiProviderRow[]>(
    () =>
      providersData
        ? dedupeProviderRows(
            providersData.providers.filter(providerEnabled),
            value?.provider ?? providersData.default,
          )
        : [],
    [providersData, value?.provider],
  );
  // A remembered selection whose provider was since switched off falls back
  // to the default instead of sending to a provider the user disabled.
  const selectionDisabled = Boolean(
    value &&
      allProviders.some(
        (row) => row.name === value.provider && !providerEnabled(row),
      ),
  );
  React.useEffect(() => {
    if (enabled && selectionDisabled) onChange(null);
  }, [enabled, selectionDisabled, onChange]);
  // Favorites saved under a deduplicated alias of a native CLI provider show
  // (and unstar) under whichever row dedupe kept for it.
  const providerAliases = React.useMemo(
    () => dedupedProviderAliases(allProviders, providers),
    [allProviders, providers],
  );
  const shownFavorites = React.useMemo(
    () => canonicalModelFavorites(favorites, providerAliases),
    [favorites, providerAliases],
  );
  const toggleShownFavorite = (provider: string, model: string) => {
    const stored = storedModelFavoritesFor(
      favorites,
      providerAliases,
      provider,
      model,
    );
    if (stored.length === 0) {
      toggleFavorite({ provider, model });
      return;
    }
    for (const entry of stored) toggleFavorite(entry);
  };
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

  useRestoreProviderModelChecks(providers, enabled);
  const modelQueries = useQueries({
    queries: providers.map((provider) =>
      providerModelsQueryOptions(dataMode, daemonSession, provider),
    ),
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
      // Not an embedding or speech model that happens to be listed first. An
      // HTTP provider none of whose models can run the assistant gets no
      // automatic pick: the picker says why and still lets the user choose
      // one through Show all models.
      const first = isCliProvider(fallbackProvider)
        ? models[0]
        : models.find(httpModelRunsAssistant);
      if (!first) return;
      onChange({
        provider: fallbackProvider.name,
        model: first.id,
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
      // `default` is Kassiber's "send no model" sentinel for CLI providers;
      // name it rather than showing the sentinel as if it were a model id.
      const labelled = isCliProvider(provider)
        ? providerModels.map((model) =>
            model.id === "default" && !model.display_name
              ? { ...model, display_name: t("modelPicker.cliDefault") }
              : model,
          )
        : providerModels;
      return { provider, models: labelled };
    });
  }, [providers, modelsByProvider, value, t]);
  // Models the user chose stay listed whatever the filter thinks of them.
  const keptModelKeys = React.useMemo(
    () =>
      new Set(
        shownFavorites.map((favorite) =>
          modelPickerModelKey(favorite.provider, favorite.model),
        ),
      ),
    [shownFavorites],
  );
  const visibleGroups = React.useMemo(
    () =>
      groupedRows.map(({ provider, models: providerModels }) => {
        if (isCliProvider(provider)) {
          return { provider, models: providerModels, hiddenCount: 0 };
        }
        const usable = providerModels.filter(
          (model) =>
            httpModelRunsAssistant(model) ||
            model.id === provider.default_model ||
            (value?.provider === provider.name && value.model === model.id) ||
            keptModelKeys.has(modelPickerModelKey(provider.name, model.id)),
        );
        return {
          provider,
          models: showAllModels ? providerModels : usable,
          // What the filter takes out, shown or not, so it can be undone.
          hiddenCount: providerModels.length - usable.length,
        };
      }),
    [groupedRows, keptModelKeys, showAllModels, value],
  );

  const selectedModelRow = value
    ? groupedRows
        .find((group) => group.provider.name === value.provider)
        ?.models.find((model) => model.id === value.model)
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

  // Only the levels the selected model advertises; none are made up for a
  // model whose levels are not known yet.
  const advertisedEfforts = React.useMemo(
    () => selectedModelReasoningEfforts({ selection: value, providers, models }),
    [value, providers, models],
  );
  const effortOptions = React.useMemo<AssistantThinkingEffort[]>(
    () => KNOWN_EFFORTS.filter((effort) => advertisedEfforts.includes(effort)),
    [advertisedEfforts],
  );
  // A Check models result lists the model, so what it advertises is all there is.
  const effortLevelsKnown = Boolean(
    value && models.some((model) => model.id === value.model),
  );

  const fastSupport = selectedModelFastMode({ selection: value, providers, models });
  const showEffortControl = showThinkingEffort;
  const showFastControl = fastSupport.supported;
  const fastOn = showFastControl && fastMode;
  const fastHint =
    fastSupport.runtime === "claude"
      ? t("composer.fastMode.claudeHint")
      : (fastSupport.description ?? t("composer.fastMode.hint"));
  const showPermission = Boolean(onPermissionModeChange);
  const fullAccess =
    showPermission && permissionModeOption(permissionMode).value === "full_access";
  // The column and footer stay beside the list while the popover is open, so
  // picking a model moves on to its levels instead of closing.
  const docked = showEffortControl || showFastControl || showPermission;

  // If a model switch leaves the current level unsupported, drop back to auto.
  // A model whose levels are not known yet keeps the stored level.
  React.useEffect(() => {
    if (!showThinkingEffort || !onThinkingEffortChange) return;
    if (effortOptions.length === 0) return;
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

  // A docked pick waiting for the host to commit the picked model.
  const pendingPickRef = React.useRef<{ provider: string; model: string } | null>(
    null,
  );
  const setPickerOpen = (next: boolean) => {
    onOpenChange?.(next);
    if (controlledOpen === undefined) setUncontrolledOpen(next);
    if (!next) {
      pendingPickRef.current = null;
      acknowledgeProvider.reset();
    }
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

  /**
   * Focuses the selected reasoning level, else the first one (T3 Code's
   * `focusEffort`). False when the column has no level to focus.
   */
  const focusEffort = () => {
    const column = contentRef.current?.querySelector("[data-model-picker-effort]");
    const level =
      column?.querySelector<HTMLElement>(
        '[data-effort-level][aria-checked="true"]:not(:disabled)',
      ) ?? column?.querySelector<HTMLElement>("[data-effort-level]:not(:disabled)");
    level?.focus({ preventScroll: true });
    level?.scrollIntoView?.({ block: "nearest" });
    return level != null;
  };
  const focusSearch = () => {
    searchInputRef.current?.focus({ preventScroll: true });
  };

  // A docked pick moves to the new model's levels once the host commits it;
  // a model without levels has nothing left to choose here, so the pick
  // closes the picker.
  const settlePick = () => {
    pendingPickRef.current = null;
    if (!focusEffort()) setPickerOpen(false);
  };
  const settlePickRef = React.useRef(settlePick);
  settlePickRef.current = settlePick;
  React.useLayoutEffect(() => {
    const pending = pendingPickRef.current;
    if (
      pending &&
      value?.provider === pending.provider &&
      value.model === pending.model
    ) {
      settlePickRef.current();
    }
  }, [value, showEffortControl, effortOptions]);

  const selectModel = async (provider: AiProviderRow, model: string) => {
    if (!(await ensureAcknowledged(provider, "modelPicker.remoteConfirm"))) {
      return;
    }
    const unchanged = value?.provider === provider.name && value.model === model;
    pendingPickRef.current = docked ? { provider: provider.name, model } : null;
    onChange({ provider: provider.name, model });
    if (!docked) {
      setPickerOpen(false);
      return;
    }
    // The current model's column is already rendered; no commit will follow.
    if (unchanged) settlePick();
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

  // Report the open picker from state, not from the open handlers: the
  // composer can open it itself (Mod+Shift+M) through the controlled prop.
  React.useEffect(() => {
    onOverlayOpenChange?.(open);
  }, [onOverlayOpenChange, open]);
  React.useEffect(
    () => () => onOverlayOpenChange?.(false),
    [onOverlayOpenChange],
  );

  const pickerShortcut = formatShortcut(MODEL_PICKER_SHORTCUT);
  const effortShortcut = formatShortcut(MODEL_PICKER_EFFORT_SHORTCUT);
  const effortLabel =
    showEffortControl && thinkingEffort !== "auto"
      ? t(`composer.effort.${thinkingEffort}`)
      : null;
  // What the trigger adds after the model, for the tooltip and screen readers.
  const triggerDetail = [
    effortLabel ? `${t("composer.reasoningEffort")}: ${effortLabel}` : null,
    fastOn ? t("composer.fastMode.fast") : null,
    fullAccess
      ? `${t("composer.permission.label")}: ${t("composer.permission.full_access.label")}`
      : null,
  ]
    .filter(Boolean)
    .join(", ");
  const modelLabel = value
    ? selectedModelRow?.display_name || value.model
    : triggerLabel;
  const FullAccessIcon = permissionModeOption("full_access").icon;
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
                ref={triggerRef}
                aria-label={`${t("modelPicker.models")}: ${triggerLabel}${
                  postureHint ? ` (${postureHint})` : ""
                }${triggerDetail ? `, ${triggerDetail}` : ""}`}
                data-chat-provider-model-picker
                data-fast-mode={fastOn || undefined}
                className="min-w-0 max-w-full shrink justify-start"
                onClick={(event) => {
                  // Mod+Shift+K aimed at the open picker moves to its levels
                  // instead of closing it.
                  if (!open || !takeOpenToEffort(triggerRef.current)) return;
                  event.preventDefault();
                  if (!focusEffort()) focusSearch();
                }}
              >
                {currentProvider ? (
                  <ProviderGlyph provider={currentProvider} className="size-4" />
                ) : null}
                <span className="min-w-0 truncate">{triggerLabel}</span>
                {triggerPosture ? <PostureBadge posture={triggerPosture} /> : null}
                {effortLabel || fastOn ? (
                  <span
                    className="flex shrink-0 items-center gap-1 text-muted-foreground"
                    data-composer-traits
                  >
                    {effortLabel ? (
                      <span className="max-w-24 truncate">{effortLabel}</span>
                    ) : null}
                    {fastOn ? (
                      // A bolt marks Fast, as on T3 Code's trigger.
                      <Zap
                        className="size-3.5 fill-current text-amber-500"
                        aria-hidden="true"
                        data-fast-mode-indicator
                      />
                    ) : null}
                  </span>
                ) : null}
                {fullAccess ? (
                  // The mode moved into the picker; Full access, which also
                  // runs network actions without asking, stays in sight, named
                  // where the toolbar has room.
                  <span
                    className="flex shrink-0 items-center gap-1"
                    aria-hidden="true"
                    data-full-access-indicator
                  >
                    <FullAccessIcon className="size-3.5 text-amber-600 dark:text-amber-400" />
                    <span className="hidden text-xs text-amber-700 @sm/composer-toolbar:inline dark:text-amber-400">
                      {t("composer.permission.full_access.label")}
                    </span>
                  </span>
                ) : null}
                <ComposerControlChevron />
              </ComposerControl>
            </PopoverTrigger>
          </TooltipTrigger>
          <TooltipContent side="top" sideOffset={6}>
            <span className="flex items-center gap-2">
              <span>
                {triggerLabel}
                {postureHint ? ` · ${postureHint}` : ""}
                {triggerDetail ? ` · ${triggerDetail}` : ""}
              </span>
              <span className="opacity-60">{pickerShortcut}</span>
              {showEffortControl ? (
                <span className="opacity-60">
                  {t("composer.reasoning")} {effortShortcut}
                </span>
              ) : null}
            </span>
          </TooltipContent>
        </Tooltip>
        <PopoverContent
          ref={contentRef}
          align="start"
          side="top"
          sideOffset={8}
          className={cn(
            "overflow-hidden rounded-xl p-0",
            showEffortControl
              ? "w-[min(36rem,calc(100vw-2rem))]"
              : "w-[min(25rem,calc(100vw-2rem))]",
          )}
          onOpenAutoFocus={(event) => {
            // Focus the search field, as T3 Code does, so typing filters and
            // the arrow keys drive the list straight away; Mod+Shift+K lands
            // on the reasoning levels instead.
            event.preventDefault();
            if (takeOpenToEffort(triggerRef.current) && focusEffort()) return;
            focusSearch();
          }}
        >
          <div className="flex max-h-(--radix-popover-content-available-height) flex-col">
            <div className="flex min-h-0 flex-row">
              <ModelPickerContent
                groups={visibleGroups}
                value={value}
                favorites={shownFavorites}
                showAllModels={showAllModels}
                onShowAllModelsChange={setShowAllModels}
                discoveryByProvider={discoveryByProvider}
                checkDisabled={acknowledgeProvider.isPending}
                selectDisabled={acknowledgeProvider.isPending}
                footerError={ackError}
                searchInputRef={searchInputRef}
                onSelect={(provider, model) => void selectModel(provider, model)}
                onToggleFavorite={toggleShownFavorite}
                onCheckModels={(provider) => void checkModels(provider)}
                docked={docked}
                onFocusEffort={
                  showEffortControl ? () => void focusEffort() : undefined
                }
              />
              {showEffortControl ? (
                <ModelPickerEffortColumn
                  modelLabel={modelLabel}
                  levels={effortOptions}
                  levelsKnown={effortLevelsKnown}
                  value={thinkingEffort}
                  onChange={onThinkingEffortChange}
                  onFocusSearch={focusSearch}
                />
              ) : null}
            </div>
            <ModelPickerFooter
              fast={
                showFastControl
                  ? { on: fastOn, hint: fastHint, onChange: onFastModeChange }
                  : null
              }
              permission={
                onPermissionModeChange
                  ? { value: permissionMode, onChange: onPermissionModeChange }
                  : null
              }
            />
          </div>
        </PopoverContent>
      </Popover>
    </TooltipProvider>
  );
}
