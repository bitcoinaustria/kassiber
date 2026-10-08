/**
 * One selectable model in the picker list.
 *
 * Adapted from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
 * `apps/web/src/components/chat/ModelListRow.tsx`. Kassiber adds the privacy
 * posture badge to every row and renders a plain `role="option"` row driven by
 * the search input's `aria-activedescendant` instead of a Base UI combobox item.
 */

import * as React from "react";
import { useTranslation } from "react-i18next";
import { Check, Star } from "lucide-react";

import { Kbd } from "@/components/ui/kbd";
import type { AiModelRow, AiProviderKind, AiProviderRow } from "@/lib/aiCapabilities";
import { cn } from "@/lib/utils";
import { PostureBadge, ProviderGlyph } from "./ProviderGlyph";
import { providerDisplayName } from "./providerIdentity";

export const ModelListRow = React.memo(function ModelListRow({
  id,
  provider,
  model,
  posture,
  isSelected,
  isHighlighted,
  isFavorite,
  showProvider,
  jumpLabel,
  disabled,
  onSelect,
  onHighlight,
  onToggleFavorite,
}: {
  id: string;
  provider: AiProviderRow;
  model: AiModelRow;
  posture: AiProviderKind;
  isSelected: boolean;
  isHighlighted: boolean;
  isFavorite: boolean;
  /** Name the provider on a second line (favorites and search span providers). */
  showProvider: boolean;
  jumpLabel: string | null;
  disabled: boolean;
  onSelect: () => void;
  onHighlight: () => void;
  onToggleFavorite: () => void;
}) {
  const { t } = useTranslation("assistant");
  const name = model.display_name || model.id;
  const favoriteLabel = isFavorite
    ? t("modelPicker.removeFavorite")
    : t("modelPicker.addFavorite");
  const showIdLine = Boolean(model.display_name) && model.display_name !== model.id;

  return (
    <div
      id={id}
      role="option"
      // Highlight follows the search field's `aria-activedescendant`, as in
      // the book switcher; the model in use is marked `aria-current`.
      aria-selected={isHighlighted}
      aria-current={isSelected || undefined}
      aria-disabled={disabled || undefined}
      data-highlighted={isHighlighted || undefined}
      data-selected={isSelected || undefined}
      data-model-picker-row
      onMouseMove={isHighlighted ? undefined : onHighlight}
      // Keep focus in the search field so the keyboard keeps driving the list.
      onMouseDown={(event) => event.preventDefault()}
      onClick={disabled ? undefined : onSelect}
      className={cn(
        "group relative flex min-h-8 w-full min-w-0 cursor-pointer items-center gap-2 rounded-md px-2 py-1.5 text-sm outline-none select-none",
        "data-[selected]:bg-foreground/[0.06] data-[highlighted]:bg-accent data-[highlighted]:text-accent-foreground",
        disabled && "cursor-not-allowed opacity-50",
      )}
    >
      <div className="min-w-0 flex-1 text-left">
        <div className="flex min-w-0 items-center gap-1.5">
          <span className="min-w-0 truncate text-xs font-medium leading-snug">
            {name}
          </span>
          <PostureBadge posture={posture} title={model.privacy_reason} />
        </div>
        {showProvider ? (
          <div className="mt-0.5 flex min-w-0 items-center gap-1.5">
            <ProviderGlyph provider={provider} className="size-3" />
            <span className="truncate text-2xs leading-snug text-muted-foreground/80">
              {providerDisplayName(provider)}
              {showIdLine ? (
                <span className="font-mono"> · {model.id}</span>
              ) : null}
            </span>
          </div>
        ) : showIdLine ? (
          <div className="mt-0.5 truncate font-mono text-2xs leading-snug text-muted-foreground/80">
            {model.id}
          </div>
        ) : null}
      </div>

      <div className="flex shrink-0 items-center gap-1.5">
        {isSelected ? <Check className="size-3.5" aria-hidden="true" /> : null}
        {jumpLabel ? <Kbd className="h-4.5 min-w-4.5 text-2xs">{jumpLabel}</Kbd> : null}
        <button
          type="button"
          tabIndex={-1}
          aria-label={favoriteLabel}
          title={favoriteLabel}
          aria-pressed={isFavorite}
          disabled={disabled}
          onMouseDown={(event) => event.preventDefault()}
          onClick={(event) => {
            event.stopPropagation();
            onToggleFavorite();
          }}
          className={cn(
            "-mr-1 inline-flex size-6 items-center justify-center rounded-md text-muted-foreground/60 outline-none transition-colors hover:bg-foreground/10 hover:text-foreground disabled:pointer-events-none",
            !isFavorite && "opacity-0 group-hover:opacity-100 group-data-[highlighted]:opacity-100",
          )}
        >
          <Star
            className={cn(
              "size-3.5",
              isFavorite && "fill-current text-amber-500",
            )}
            aria-hidden="true"
          />
        </button>
      </div>
    </div>
  );
});
