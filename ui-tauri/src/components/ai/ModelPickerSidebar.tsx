/**
 * Vertical provider rail for the model picker: a Favorites entry, then one
 * button per configured provider. Scrolls (with its scrollbar hidden) once
 * there are more providers than fit, so ten or more stay usable.
 *
 * Adapted from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
 * `apps/web/src/components/chat/ModelPickerSidebar.tsx`. Kassiber adds a
 * privacy-posture dot to each provider and names the posture in its tooltip.
 */

import * as React from "react";
import { useTranslation } from "react-i18next";
import { Star } from "lucide-react";

import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import type { AiProviderRow } from "@/lib/aiCapabilities";
import { isImeKeyEvent } from "@/lib/imeKeyEvent";
import { cn } from "@/lib/utils";
import {
  FAVORITES_VIEW,
  modelPickerViewKey,
  type ModelPickerView,
} from "./modelPickerKeys";
import { ProviderGlyph } from "./ProviderGlyph";
import { providerDisplayName } from "./providerIdentity";

const RAIL_BUTTON_CLASS =
  "relative isolate flex aspect-square w-full cursor-pointer items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-foreground/10 hover:text-foreground focus-visible:bg-foreground/10 focus-visible:ring-2 focus-visible:ring-ring aria-pressed:text-foreground";

export const ModelPickerSidebar = React.memo(function ModelPickerSidebar({
  providers,
  view,
  staleProviders,
  onSelectView,
  onFocusSearch,
}: {
  providers: AiProviderRow[];
  view: ModelPickerView;
  /** Providers whose last discovery failed and now show a stale inventory. */
  staleProviders: ReadonlySet<string>;
  onSelectView: (view: ModelPickerView) => void;
  onFocusSearch: () => void;
}) {
  const { t } = useTranslation("assistant");
  const contentRef = React.useRef<HTMLDivElement>(null);
  const activeKey = modelPickerViewKey(view);
  const [indicatorTop, setIndicatorTop] = React.useState<number | null>(null);

  React.useLayoutEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    const item = Array.from(
      content.querySelectorAll<HTMLElement>("[data-model-picker-view]"),
    ).find((element) => element.dataset.modelPickerView === activeKey);
    if (!item) {
      setIndicatorTop(null);
      return;
    }
    setIndicatorTop(item.offsetTop + item.offsetHeight / 2 - 10);
    item.scrollIntoView?.({ block: "nearest" });
  }, [activeKey, providers]);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (isImeKeyEvent(event)) return;
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    if (event.key === "ArrowRight") {
      event.preventDefault();
      onFocusSearch();
      return;
    }
    if (
      event.key !== "ArrowDown" &&
      event.key !== "ArrowUp" &&
      event.key !== "Home" &&
      event.key !== "End"
    ) {
      return;
    }
    const buttons = Array.from(
      event.currentTarget.querySelectorAll<HTMLButtonElement>(
        "button[data-model-picker-rail-button]",
      ),
    );
    if (buttons.length === 0) return;
    event.preventDefault();
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? buttons.length - 1
          : (current + (event.key === "ArrowDown" ? 1 : -1) + buttons.length) %
            buttons.length;
    buttons[next]?.focus();
  };

  const railButton = (
    key: string,
    label: string,
    tooltip: string,
    content: React.ReactNode,
    onClick: () => void,
  ) => {
    const selected = key === activeKey;
    return (
      <div key={key} className="relative w-full" data-model-picker-view={key}>
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              data-model-picker-rail-button
              className={cn(RAIL_BUTTON_CLASS, selected && "bg-background shadow-xs")}
              // Roving focus: only the selected entry is in the tab order.
              tabIndex={selected ? 0 : -1}
              aria-pressed={selected}
              aria-label={label}
              onClick={onClick}
            >
              {content}
            </button>
          </TooltipTrigger>
          <TooltipContent side="left" sideOffset={8}>
            {tooltip}
          </TooltipContent>
        </Tooltip>
      </div>
    );
  };

  return (
    <div
      role="toolbar"
      aria-orientation="vertical"
      aria-label={t("modelPicker.providers")}
      data-model-picker-sidebar
      className="relative w-11 shrink-0 border-r border-border/60 bg-muted/30"
      onKeyDown={handleKeyDown}
    >
      {/* Absolutely filled so the rail never sets the popover's height: the
          model list does, and the rail scrolls inside it. */}
      <div className="absolute inset-0 overflow-y-auto overscroll-contain [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        <div ref={contentRef} className="relative flex min-h-full flex-col gap-1 p-1">
          {indicatorTop !== null ? (
            <div
              aria-hidden="true"
              data-model-picker-selected-indicator
              className="pointer-events-none absolute right-0 z-10 h-5 w-0.75 rounded-l-full bg-primary transition-[top] duration-200 ease-out"
              style={{ top: indicatorTop }}
            />
          ) : null}
          {railButton(
            FAVORITES_VIEW,
            t("modelPicker.favorites"),
            t("modelPicker.favorites"),
            <Star className="size-4.5 fill-current" aria-hidden="true" />,
            () => onSelectView(FAVORITES_VIEW),
          )}
          <div className="mx-1 border-b border-border/70" aria-hidden="true" />
          {providers.map((provider) => {
            const name = providerDisplayName(provider);
            const stale = staleProviders.has(provider.name);
            const tooltip = [
              name,
              t(`modelPicker.posture.${provider.kind}`),
              stale ? t("modelPicker.staleShort") : null,
            ]
              .filter(Boolean)
              .join(" · ");
            return railButton(
              modelPickerViewKey({ provider: provider.name }),
              `${name} · ${t(`modelPicker.posture.${provider.kind}`)}`,
              tooltip,
              <ProviderGlyph
                provider={provider}
                className="size-5"
                // A stale inventory reads as a warning, like an off-device one.
                dotPosture={stale ? "remote" : provider.kind}
              />,
              () => onSelectView({ provider: provider.name }),
            );
          })}
        </div>
      </div>
    </div>
  );
});
