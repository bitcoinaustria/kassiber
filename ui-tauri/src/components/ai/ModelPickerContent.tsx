/**
 * Body of the provider/model picker: provider rail, search, provider header
 * with the explicit **Check models** action, and a keyboard-driven model list.
 *
 * Adapted from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
 * `apps/web/src/components/chat/ModelPickerContent.tsx`. Same interaction
 * model — search field keeps focus, ArrowUp/ArrowDown move the highlight,
 * Enter picks, ArrowLeft / Shift+Tab reach the rail, Mod+Shift+ArrowUp/Down
 * switch provider, Mod+1…9 pick the nth row, starred models collect under
 * Favorites and a query searches every provider at once.
 *
 * Kassiber differences: nothing here contacts a provider. Search, favorites
 * and provider switching only read configured defaults and models a previous
 * explicit check already returned; discovery runs solely from the Check
 * models button. Every row carries its privacy posture.
 */

import * as React from "react";
import { useTranslation } from "react-i18next";
import { RefreshCw, Search, Star } from "lucide-react";

import { Button } from "@/components/ui/button";
import type {
  AiModelRow,
  AiModelsListData,
  AiProviderKind,
  AiProviderRow,
  AssistantModelSelection,
} from "@/lib/aiCapabilities";
import { formatShortcut } from "@/lib/shortcutLabel";
import { cn } from "@/lib/utils";
import { ModelListRow } from "./ModelListRow";
import { ModelPickerSidebar } from "./ModelPickerSidebar";
import {
  FAVORITES_VIEW,
  initialModelPickerView,
  modelPickerModelKey,
  modelPickerViewKey,
  type ModelPickerView,
} from "./modelPickerKeys";
import { rankModelPickerItems } from "./modelPickerSearch";
import { PostureBadge, ProviderGlyph } from "./ProviderGlyph";
import { isCliProvider, providerDisplayName } from "./providerIdentity";
import { modelPrivacyPosture, sortModelRowsByPosture } from "./providerModelSearch";

export interface ModelPickerProviderGroup {
  provider: AiProviderRow;
  models: AiModelRow[];
}

export interface ModelPickerDiscoveryState {
  isFetching: boolean;
  error: Error | null;
  snapshot: AiModelsListData | undefined;
}

interface PickerItem {
  key: string;
  provider: AiProviderRow;
  model: AiModelRow;
  posture: AiProviderKind;
  isFavorite: boolean;
}

const MAX_JUMP_SHORTCUTS = 9;

/** Top/bottom edge fades for a scroll container, as in T3 Code's model list. */
function useScrollFade(ref: React.RefObject<HTMLElement | null>) {
  const [fade, setFade] = React.useState({ top: false, bottom: false });
  const update = React.useCallback(() => {
    const element = ref.current;
    if (!element) return;
    const max = Math.max(0, element.scrollHeight - element.clientHeight);
    const next = { top: element.scrollTop > 1, bottom: max - element.scrollTop > 1 };
    setFade((current) =>
      current.top === next.top && current.bottom === next.bottom ? current : next,
    );
  }, [ref]);
  return { fade, update };
}

export function ModelPickerContent({
  groups,
  value,
  favorites,
  localOnly,
  onLocalOnlyChange,
  discoveryByProvider,
  checkDisabled,
  selectDisabled,
  footerError,
  searchInputRef,
  onSelect,
  onToggleFavorite,
  onCheckModels,
}: {
  /** Providers and their known models, already narrowed by the Local filter. */
  groups: ModelPickerProviderGroup[];
  value: AssistantModelSelection;
  favorites: NonNullable<AssistantModelSelection>[];
  localOnly: boolean;
  onLocalOnlyChange: (localOnly: boolean) => void;
  discoveryByProvider: ReadonlyMap<string, ModelPickerDiscoveryState>;
  checkDisabled: boolean;
  selectDisabled: boolean;
  footerError: string | null;
  searchInputRef: React.RefObject<HTMLInputElement | null>;
  onSelect: (provider: AiProviderRow, model: string) => void;
  onToggleFavorite: (provider: string, model: string) => void;
  onCheckModels: (provider: AiProviderRow) => void;
}) {
  const { t } = useTranslation("assistant");
  const listId = React.useId();
  const [view, setView] = React.useState<ModelPickerView>(() =>
    initialModelPickerView({ groups, value, favorites }),
  );
  const [query, setQuery] = React.useState("");
  const [highlightedKey, setHighlightedKey] = React.useState<string | null>(null);
  const [heldHeight, setHeldHeight] = React.useState<number | null>(null);
  const rootRef = React.useRef<HTMLDivElement>(null);
  const listRef = React.useRef<HTMLDivElement>(null);
  const scrollHighlightRef = React.useRef(false);

  const isSearching = query.trim().length > 0;
  const favoriteKeys = React.useMemo(
    () =>
      new Set(favorites.map((favorite) => modelPickerModelKey(favorite.provider, favorite.model))),
    [favorites],
  );
  const groupByName = React.useMemo(
    () => new Map(groups.map((group) => [group.provider.name, group])),
    [groups],
  );

  // A provider hidden by the Local filter (or removed in Settings) cannot stay
  // the active view; fall back to the first remaining provider.
  const activeView = React.useMemo<ModelPickerView>(
    () =>
      view === FAVORITES_VIEW || groupByName.has(view.provider)
        ? view
        : groups[0]
          ? { provider: groups[0].provider.name }
          : FAVORITES_VIEW,
    [groupByName, groups, view],
  );
  const activeGroup =
    activeView === FAVORITES_VIEW ? undefined : groupByName.get(activeView.provider);

  const toItem = React.useCallback(
    (provider: AiProviderRow, model: AiModelRow): PickerItem => {
      const key = modelPickerModelKey(provider.name, model.id);
      return {
        key,
        provider,
        model,
        posture: modelPrivacyPosture(provider, model),
        isFavorite: favoriteKeys.has(key),
      };
    },
    [favoriteKeys],
  );

  const groupItems = React.useCallback(
    (group: ModelPickerProviderGroup): PickerItem[] => {
      // Favorites first, then privacy-first (`sortModelRowsByPosture` is
      // stable), so posture stays the primary order.
      const starredFirst = [...group.models].sort(
        (a, b) =>
          Number(favoriteKeys.has(modelPickerModelKey(group.provider.name, b.id))) -
          Number(favoriteKeys.has(modelPickerModelKey(group.provider.name, a.id))),
      );
      return sortModelRowsByPosture(group.provider, starredFirst).map((model) =>
        toItem(group.provider, model),
      );
    },
    [favoriteKeys, toItem],
  );

  const favoriteItems = React.useMemo(() => {
    const items: PickerItem[] = [];
    for (const group of groups) {
      for (const favorite of favorites) {
        if (favorite.provider !== group.provider.name) continue;
        const model =
          group.models.find((row) => row.id === favorite.model) ?? { id: favorite.model };
        const item = toItem(group.provider, model);
        // An unverified favorite falls back to the provider's posture, so the
        // Local filter can only keep it when that posture is local too.
        if (localOnly && item.posture !== "local") continue;
        items.push(item);
      }
    }
    return items;
  }, [favorites, groups, localOnly, toItem]);

  const items = React.useMemo<PickerItem[]>(() => {
    if (isSearching) {
      const seen = new Set<string>();
      const all: PickerItem[] = [];
      for (const item of [...groups.flatMap(groupItems), ...favoriteItems]) {
        if (seen.has(item.key)) continue;
        seen.add(item.key);
        all.push(item);
      }
      return rankModelPickerItems(all, query, (item) => ({
        id: item.model.id,
        displayName: item.model.display_name,
        ownedBy: item.model.owned_by,
        sourceProvider: item.model.source_provider,
        providerName: item.provider.name,
        providerDisplayName: providerDisplayName(item.provider),
        isFavorite: item.isFavorite,
      }));
    }
    if (activeView === FAVORITES_VIEW) return favoriteItems;
    return activeGroup ? groupItems(activeGroup) : [];
  }, [activeGroup, activeView, favoriteItems, groupItems, groups, isSearching, query]);

  const selectedKey = value ? modelPickerModelKey(value.provider, value.model) : null;
  const effectiveHighlightKey =
    highlightedKey && items.some((item) => item.key === highlightedKey)
      ? highlightedKey
      : selectedKey && items.some((item) => item.key === selectedKey)
        ? selectedKey
        : (items[0]?.key ?? null);
  const highlightedIndex = items.findIndex((item) => item.key === effectiveHighlightKey);

  const jumpLabels = React.useMemo(() => {
    const labels = new Map<string, string>();
    if (selectDisabled) return labels;
    items.slice(0, MAX_JUMP_SHORTCUTS).forEach((item, index) => {
      labels.set(item.key, formatShortcut(["mod", String(index + 1)]));
    });
    return labels;
  }, [items, selectDisabled]);

  const { fade, update: updateFade } = useScrollFade(listRef);
  React.useLayoutEffect(() => updateFade(), [items, updateFade]);

  React.useEffect(() => {
    if (!scrollHighlightRef.current || !effectiveHighlightKey) return;
    scrollHighlightRef.current = false;
    document
      .getElementById(`${listId}-${effectiveHighlightKey}`)
      ?.scrollIntoView?.({ block: "nearest" });
  }, [effectiveHighlightKey, listId]);

  const focusSearch = React.useCallback(() => {
    searchInputRef.current?.focus({ preventScroll: true });
  }, [searchInputRef]);

  const selectView = React.useCallback(
    (next: ModelPickerView) => {
      setView(next);
      setQuery("");
      setHeldHeight(null);
      setHighlightedKey(null);
      window.requestAnimationFrame(focusSearch);
    },
    [focusSearch],
  );

  const pick = React.useCallback(
    (item: PickerItem | undefined) => {
      if (!item || selectDisabled) return;
      onSelect(item.provider, item.model.id);
    },
    [onSelect, selectDisabled],
  );

  const railViews = React.useMemo<ModelPickerView[]>(
    () => [FAVORITES_VIEW, ...groups.map(({ provider }) => ({ provider: provider.name }))],
    [groups],
  );

  // Picker-scoped shortcuts, captured before the app shell sees them. The
  // listener only exists while the picker is open (this content unmounts on
  // close), so Mod+digit keeps its meaning everywhere else.
  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.repeat || event.altKey) return;
      if (!(event.metaKey || event.ctrlKey)) return;
      if (event.shiftKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
        event.preventDefault();
        event.stopPropagation();
        const current = railViews.findIndex(
          (candidate) => modelPickerViewKey(candidate) === modelPickerViewKey(activeView),
        );
        const step = event.key === "ArrowDown" ? 1 : -1;
        const next =
          railViews[
            current < 0
              ? 0
              : (current + step + railViews.length) % railViews.length
          ];
        if (next) selectView(next);
        return;
      }
      if (!event.shiftKey && /^[1-9]$/.test(event.key)) {
        const item = items[Number(event.key) - 1];
        if (!item) return;
        event.preventDefault();
        event.stopPropagation();
        pick(item);
      }
    };
    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [activeView, items, pick, railViews, selectView]);

  const moveHighlight = (delta: number) => {
    if (items.length === 0) return;
    const start = highlightedIndex < 0 ? (delta > 0 ? -1 : 0) : highlightedIndex;
    const next = (start + delta + items.length) % items.length;
    scrollHighlightRef.current = true;
    setHighlightedKey(items[next].key);
  };

  const handleSearchKeyDown = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return;
    const plain = !event.altKey && !event.ctrlKey && !event.metaKey;
    if (plain && event.key === "ArrowDown") {
      event.preventDefault();
      moveHighlight(1);
      return;
    }
    if (plain && event.key === "ArrowUp") {
      event.preventDefault();
      moveHighlight(-1);
      return;
    }
    if (plain && event.key === "Enter") {
      event.preventDefault();
      pick(items[highlightedIndex]);
      return;
    }
    if (
      plain &&
      !isSearching &&
      ((event.key === "ArrowLeft" && !event.shiftKey && query.length === 0) ||
        (event.key === "Tab" && event.shiftKey))
    ) {
      const rail = rootRef.current?.querySelector("[data-model-picker-sidebar]");
      const button =
        rail?.querySelector<HTMLButtonElement>('button[aria-pressed="true"]') ??
        rail?.querySelector<HTMLButtonElement>("button");
      if (button) {
        event.preventDefault();
        button.focus();
      }
    }
  };

  const discovery = activeGroup
    ? discoveryByProvider.get(activeGroup.provider.name)
    : undefined;
  const staleProviders = React.useMemo(() => {
    const stale = new Set<string>();
    discoveryByProvider.forEach((state, name) => {
      if (state.snapshot?.stale) stale.add(name);
    });
    return stale;
  }, [discoveryByProvider]);

  const emptyMessage = (() => {
    if (groups.length === 0 && favoriteItems.length === 0) {
      return { title: t("modelPicker.noProviders"), hint: null };
    }
    if (isSearching) return { title: t("modelPicker.noMatchingModels"), hint: null };
    if (activeView === FAVORITES_VIEW) {
      return { title: t("modelPicker.noFavorites"), hint: t("modelPicker.noFavoritesHint") };
    }
    if (discovery?.isFetching) return { title: t("modelPicker.checkingModels"), hint: null };
    if (discovery?.error) return { title: discovery.error.message, hint: null };
    return {
      title: t("modelPicker.noModels"),
      hint: activeGroup
        ? t("modelPicker.noModelsHint", { provider: providerDisplayName(activeGroup.provider) })
        : null,
    };
  })();

  return (
    <div
      ref={rootRef}
      data-model-picker-content
      className="relative flex max-h-[min(23rem,calc(var(--radix-popover-content-available-height,100vh)-1rem))] min-h-[15rem] w-full flex-row overflow-hidden"
      // Hold the height from when the search started: results scroll instead
      // of the popover jumping under the pointer.
      style={heldHeight !== null ? { height: heldHeight } : undefined}
    >
      {isSearching ? null : (
        <ModelPickerSidebar
          providers={groups.map(({ provider }) => provider)}
          view={activeView}
          staleProviders={staleProviders}
          onSelectView={selectView}
          onFocusSearch={focusSearch}
        />
      )}

      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <div className="flex shrink-0 items-center gap-2 px-3 pt-2.5">
          <div className="relative min-w-0 flex-1 border-b border-border/70 pb-1.5 transition-colors focus-within:border-ring">
            <Search
              aria-hidden="true"
              className="pointer-events-none absolute top-1.5 left-0 size-4 text-muted-foreground/55"
            />
            <input
              ref={searchInputRef}
              type="text"
              role="combobox"
              aria-expanded="true"
              aria-controls={listId}
              aria-autocomplete="list"
              aria-activedescendant={
                effectiveHighlightKey ? `${listId}-${effectiveHighlightKey}` : undefined
              }
              aria-label={t("modelPicker.searchModels")}
              autoComplete="off"
              spellCheck={false}
              value={query}
              placeholder={t("modelPicker.searchModels")}
              onChange={(event) => {
                if (!isSearching) setHeldHeight(rootRef.current?.offsetHeight ?? null);
                if (event.target.value.trim().length === 0) setHeldHeight(null);
                setQuery(event.target.value);
                setHighlightedKey(null);
              }}
              onKeyDown={handleSearchKeyDown}
              className="h-6.5 w-full bg-transparent ps-6 text-sm leading-6.5 text-foreground outline-none placeholder:text-muted-foreground/60"
            />
          </div>
          <div
            className="inline-flex shrink-0 rounded-md bg-muted p-0.5 text-2xs"
            role="group"
            aria-label={t("modelPicker.privacyFilter")}
          >
            {([false, true] as const).map((local) => (
              <button
                key={String(local)}
                type="button"
                aria-pressed={localOnly === local}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => onLocalOnlyChange(local)}
                className={cn(
                  "rounded-[5px] px-1.5 py-0.5 text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
                  localOnly === local && "bg-background text-foreground shadow-xs",
                )}
              >
                {local ? t("modelPicker.localModels") : t("modelPicker.allModels")}
              </button>
            ))}
          </div>
        </div>

        {!isSearching && activeGroup ? (
          <div className="shrink-0 px-3 pt-2 pb-1">
            <div className="flex min-w-0 items-center gap-2">
              <ProviderGlyph provider={activeGroup.provider} className="size-4" />
              <div className="min-w-0 flex-1">
                <p className="flex min-w-0 items-center gap-1.5 text-xs font-medium leading-snug">
                  <span className="truncate">{providerDisplayName(activeGroup.provider)}</span>
                  <PostureBadge posture={activeGroup.provider.kind} />
                </p>
                <p className="truncate text-2xs leading-snug text-muted-foreground">
                  {isCliProvider(activeGroup.provider)
                    ? discovery?.isFetching
                      ? t("modelPicker.checkingProvider")
                      : t("modelPicker.cliChatOnly")
                    : activeGroup.provider.base_url}
                </p>
              </div>
              <Button
                type="button"
                size="xs"
                variant="ghost"
                data-model-picker-check
                className="shrink-0 text-muted-foreground hover:text-foreground"
                disabled={checkDisabled || discovery?.isFetching}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => onCheckModels(activeGroup.provider)}
                title={t("modelPicker.checkModelsHint", {
                  provider: providerDisplayName(activeGroup.provider),
                })}
              >
                <RefreshCw
                  className={cn("size-3", discovery?.isFetching && "animate-spin")}
                  aria-hidden="true"
                />
                {discovery?.isFetching
                  ? t("modelPicker.checkingModels")
                  : t("modelPicker.checkModels")}
              </Button>
            </div>
            {discovery?.error ? (
              <p role="status" className="mt-1 line-clamp-2 text-2xs text-destructive">
                {discovery.error.message}
              </p>
            ) : null}
            {discovery?.snapshot?.stale ? (
              <p className="mt-1 truncate text-2xs text-amber-600 dark:text-amber-400">
                {t("modelPicker.staleModels", {
                  error: discovery.snapshot.error?.message ?? "",
                })}
              </p>
            ) : null}
          </div>
        ) : null}

        <div
          ref={listRef}
          id={listId}
          role="listbox"
          aria-label={t("modelPicker.models")}
          onScroll={updateFade}
          className={cn(
            "kb-scroll-fade min-h-0 flex-1 overflow-x-hidden overflow-y-auto overscroll-contain px-1.5 py-1.5",
          )}
          data-fade-top={fade.top || undefined}
          data-fade-bottom={fade.bottom || undefined}
        >
          {items.length === 0 ? (
            <div className="flex flex-col items-center gap-1 px-4 py-8 text-center">
              {activeView === FAVORITES_VIEW && !isSearching ? (
                <Star className="mb-1 size-4 text-muted-foreground/60" aria-hidden="true" />
              ) : null}
              <p className="text-xs text-muted-foreground">{emptyMessage.title}</p>
              {emptyMessage.hint ? (
                <p className="max-w-56 text-2xs text-muted-foreground/70">
                  {emptyMessage.hint}
                </p>
              ) : null}
            </div>
          ) : (
            <div className="flex flex-col gap-0.5">
              {items.map((item, index) => (
                <ModelListRow
                  key={item.key}
                  id={`${listId}-${item.key}`}
                  provider={item.provider}
                  model={item.model}
                  posture={item.posture}
                  isSelected={item.key === selectedKey}
                  isHighlighted={index === highlightedIndex}
                  isFavorite={item.isFavorite}
                  showProvider={isSearching || activeView === FAVORITES_VIEW}
                  jumpLabel={jumpLabels.get(item.key) ?? null}
                  disabled={selectDisabled}
                  onSelect={() => pick(item)}
                  onHighlight={() => setHighlightedKey(item.key)}
                  onToggleFavorite={() => onToggleFavorite(item.provider.name, item.model.id)}
                />
              ))}
            </div>
          )}
        </div>

        {footerError ? (
          <div className="shrink-0 border-t border-border/60 px-3 py-2">
            <p className="text-2xs text-destructive">{footerError}</p>
          </div>
        ) : null}
      </div>
    </div>
  );
}
