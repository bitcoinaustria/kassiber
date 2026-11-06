/**
 * The model picker's docked traits: a reasoning column beside the model list
 * and a footer under both with the speed toggle and the permission mode.
 *
 * Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
 * `apps/web/src/components/chat/ComposerModelEffort.tsx` (the effort column,
 * its row navigation, the speed segments and the permissions menu). Kassiber
 * feeds it from its own capability data instead of provider option
 * descriptors: only the levels the selected model advertises, fast mode only
 * where its checked row offers it, and the permission modes in
 * `permissionMode.ts`.
 */

import * as React from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronUp, Zap } from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { isAiPermissionMode, type AiPermissionMode } from "@/daemon/stream";
import { cn } from "@/lib/utils";
import type { AssistantThinkingEffort } from "./assistantSession";
import { PERMISSION_MODE_OPTIONS, permissionModeOption } from "./permissionMode";

const COLUMN_ROW =
  "flex h-8 w-full shrink-0 cursor-pointer items-center gap-2 rounded-sm px-2 text-left text-sm text-foreground outline-none hover:bg-foreground/[0.09] focus-visible:bg-foreground/[0.09] disabled:cursor-not-allowed disabled:opacity-60";

function EffortRow(props: {
  label: string;
  /** Shown in a tooltip and read as the row's description. */
  description?: string;
  selected: boolean;
  /** The column's one tab stop: the selected level, else the first. */
  tabStop: boolean;
  isDefault?: boolean;
  disabled: boolean;
  onPick: () => void;
}) {
  const { t } = useTranslation("assistant");
  const descriptionId = React.useId();
  const row = (
    <button
      type="button"
      role="radio"
      aria-checked={props.selected}
      aria-describedby={props.description ? descriptionId : undefined}
      disabled={props.disabled}
      // The arrow keys move between rows; focus lands on a level after a model pick.
      data-effort-level
      tabIndex={props.tabStop ? 0 : -1}
      className={COLUMN_ROW}
      onClick={props.onPick}
    >
      <span className="min-w-0 flex-1 truncate">{props.label}</span>
      {props.selected ? (
        <>
          {props.isDefault ? (
            <span className="sr-only">{t("composer.effortDefault")}</span>
          ) : null}
          <Check aria-hidden="true" className="size-3.5" />
        </>
      ) : props.isDefault ? (
        <span className="text-xs text-muted-foreground/70">
          {t("composer.effortDefault")}
        </span>
      ) : null}
    </button>
  );
  if (!props.description) return row;
  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>{row}</TooltipTrigger>
        <TooltipContent side="right" sideOffset={6}>
          {props.description}
        </TooltipContent>
      </Tooltip>
      {/* Hidden text still names the description for screen readers. */}
      <span id={descriptionId} hidden>
        {props.description}
      </span>
    </>
  );
}

/**
 * Up/Down, Home and End move between levels and choose the one they land
 * on: a radio group's selection follows focus.
 */
function moveBetweenRows(event: React.KeyboardEvent<HTMLElement>) {
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  if (event.altKey || event.ctrlKey || event.metaKey) return;
  const rows = Array.from(
    event.currentTarget.querySelectorAll<HTMLButtonElement>(
      "[data-effort-level]:not(:disabled)",
    ),
  );
  const index = rows.findIndex((row) => row === document.activeElement);
  const next =
    event.key === "Home"
      ? rows[0]
      : event.key === "End"
        ? rows[rows.length - 1]
        : rows[
            Math.min(
              rows.length - 1,
              Math.max(0, index + (event.key === "ArrowDown" ? 1 : -1)),
            )
          ];
  if (!next) return;
  event.preventDefault();
  event.stopPropagation();
  next.focus();
  next.scrollIntoView?.({ block: "nearest" });
  next.click();
}

/**
 * The reasoning column: the selected model's name over its levels. Auto
 * (send no level) always leads as the default. `levels` is what the model
 * advertises; when it advertises none yet, only Auto and a stored choice are
 * listed, with a hint that Check models names the rest unless the model was
 * already checked (`levelsKnown`).
 */
export function ModelPickerEffortColumn({
  modelLabel,
  levels,
  levelsKnown,
  value,
  onChange,
  onFocusSearch,
}: {
  modelLabel: string;
  levels: readonly AssistantThinkingEffort[];
  /** Whether a Check models result lists this model. */
  levelsKnown: boolean;
  value: AssistantThinkingEffort;
  /** Omitted while a reply streams: the column shows the level, locked. */
  onChange?: (effort: AssistantThinkingEffort) => void;
  onFocusSearch: () => void;
}) {
  const { t } = useTranslation("assistant");
  const shown: AssistantThinkingEffort[] = ["auto", ...levels];
  // A level chosen before the model's levels were known stays visible.
  if (!shown.includes(value)) shown.push(value);
  const disabled = !onChange;
  return (
    <div
      data-model-picker-effort
      className="flex min-h-0 w-44 shrink-0 flex-col border-l border-border/70 bg-muted/40"
      onKeyDown={(event) => {
        if (
          event.key !== "ArrowLeft" ||
          event.altKey ||
          event.ctrlKey ||
          event.metaKey
        ) {
          return;
        }
        event.preventDefault();
        event.stopPropagation();
        onFocusSearch();
      }}
    >
      {/* Matches the search row's box, so one rule runs across the top. */}
      <div className="shrink-0 px-3 pt-2.5">
        <div className="flex h-8 items-center gap-2 border-b border-border/70 pb-1.5 text-sm">
          <span className="shrink-0 text-muted-foreground">
            {t("composer.reasoning")}
          </span>
          <span
            className="ms-auto min-w-0 truncate text-xs text-muted-foreground"
            title={modelLabel}
          >
            {modelLabel}
          </span>
        </div>
      </div>
      <div
        role="radiogroup"
        aria-label={t("composer.reasoningEffort")}
        className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto overscroll-contain px-2 pt-2 pb-1.5"
        onKeyDownCapture={moveBetweenRows}
      >
        {shown.map((effort) => (
          <EffortRow
            key={effort}
            label={t(`composer.effort.${effort}`)}
            description={effort === "auto" ? t("composer.effortAutoHint") : undefined}
            isDefault={effort === "auto"}
            selected={effort === value}
            tabStop={effort === value}
            disabled={disabled}
            onPick={() => {
              if (effort !== value) onChange?.(effort);
            }}
          />
        ))}
        {levels.length === 0 && !levelsKnown ? (
          <p className="px-2 pt-1 text-2xs text-muted-foreground/80">
            {t("composer.effortLevelsUnknown")}
          </p>
        ) : null}
      </div>
    </div>
  );
}

const FOOTER_SEGMENT =
  "flex h-6 cursor-pointer items-center justify-center gap-1 px-2 text-xs text-muted-foreground outline-none first:rounded-l-md last:rounded-r-md hover:text-foreground disabled:cursor-not-allowed disabled:opacity-60 data-[checked]:bg-background data-[checked]:text-foreground data-[checked]:shadow-xs focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring";

const SPEED_KEYS = ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"];

/**
 * Standard | Fast, one click away in the footer. One tab stop; the arrow keys
 * switch between the two. The billing note stays visible while Fast is on.
 */
function SpeedControl({
  fastMode,
  hint,
  onChange,
}: {
  fastMode: boolean;
  hint: string;
  onChange?: (on: boolean) => void;
}) {
  const { t } = useTranslation("assistant");
  const hintId = React.useId();
  return (
    <>
      <div
        role="radiogroup"
        aria-label={t("composer.fastMode.speed")}
        data-fast-mode-toggle
        className="flex rounded-lg bg-muted p-0.5"
        onKeyDown={(event) => {
          if (!onChange || !SPEED_KEYS.includes(event.key)) return;
          if (event.altKey || event.ctrlKey || event.metaKey) return;
          event.preventDefault();
          event.stopPropagation();
          event.currentTarget
            .querySelector<HTMLButtonElement>('[role="radio"][aria-checked="false"]')
            ?.focus();
          onChange(!fastMode);
        }}
      >
        {([false, true] as const).map((fast) => (
          <button
            key={String(fast)}
            type="button"
            role="radio"
            aria-checked={fastMode === fast}
            aria-describedby={fast ? hintId : undefined}
            data-checked={fastMode === fast || undefined}
            disabled={!onChange}
            tabIndex={fastMode === fast ? 0 : -1}
            title={fast && !fastMode ? hint : undefined}
            className={FOOTER_SEGMENT}
            onClick={() => {
              if (fastMode !== fast) onChange?.(fast);
            }}
          >
            {fast ? <Zap aria-hidden="true" className="size-3 fill-current" /> : null}
            {fast ? t("composer.fastMode.fast") : t("composer.fastMode.standard")}
          </button>
        ))}
      </div>
      <p
        id={hintId}
        data-fast-mode-hint
        hidden={!fastMode}
        className="min-w-0 text-2xs text-muted-foreground"
      >
        {hint}
      </p>
    </>
  );
}

const WARNING_TONE =
  "text-amber-700 hover:text-amber-800 dark:text-amber-400 dark:hover:text-amber-300";

/**
 * Permissions change rarely, so the footer names the mode and nests the
 * choice in a menu, as T3 Code's footer does. The menu is the one the
 * composer toolbar had: each mode with its description, Full access in a
 * warning tone.
 */
export function PermissionModeMenu({
  value,
  onChange,
}: {
  value: AiPermissionMode;
  onChange: (mode: AiPermissionMode) => void;
}) {
  const { t } = useTranslation("assistant");
  const current = permissionModeOption(value);
  const CurrentIcon = current.icon;
  const warning = current.value === "full_access";
  const title = t("composer.permission.label");
  return (
    <DropdownMenu modal={false}>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              aria-label={`${title}: ${t(current.labelKey)}`}
              data-composer-option="permission-mode"
              data-value={current.value}
              className={cn(
                "ms-auto flex h-7 cursor-pointer items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground outline-none hover:bg-foreground/[0.09] hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring data-[state=open]:bg-foreground/[0.09] data-[state=open]:text-foreground",
                warning && WARNING_TONE,
              )}
            >
              <CurrentIcon aria-hidden="true" className="size-3.5" />
              {t(current.labelKey)}
              <ChevronUp aria-hidden="true" className="size-3 opacity-70" />
            </button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent side="top" sideOffset={6} className="max-w-64 text-pretty">
          {t(current.descriptionKey)}
        </TooltipContent>
      </Tooltip>
      <DropdownMenuContent
        side="top"
        align="end"
        className="w-72"
      >
        <DropdownMenuLabel className="text-xs font-medium text-muted-foreground">
          {title}
        </DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={current.value}
          onValueChange={(next) => {
            if (isAiPermissionMode(next)) onChange(next);
          }}
        >
          {PERMISSION_MODE_OPTIONS.map((option) => {
            const OptionIcon = option.icon;
            return (
              <DropdownMenuRadioItem
                key={option.value}
                value={option.value}
                className="items-start py-2"
              >
                <span className="grid min-w-0 flex-1 gap-0.5">
                  <span className="inline-flex items-center gap-1.5 font-medium text-foreground">
                    <OptionIcon
                      className={cn(
                        "size-3.5 shrink-0",
                        option.value === "full_access"
                          ? "text-amber-600 dark:text-amber-400"
                          : "text-muted-foreground",
                      )}
                      aria-hidden="true"
                    />
                    {t(option.labelKey)}
                  </span>
                  <span className="text-xs leading-4 text-muted-foreground">
                    {t(option.descriptionKey)}
                  </span>
                </span>
              </DropdownMenuRadioItem>
            );
          })}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** The picker's footer: speed stays one click away, permissions one menu deeper. */
export function ModelPickerFooter({
  fast,
  permission,
}: {
  fast: { on: boolean; hint: string; onChange?: (on: boolean) => void } | null;
  permission: {
    value: AiPermissionMode;
    onChange: (mode: AiPermissionMode) => void;
  } | null;
}) {
  if (!fast && !permission) return null;
  return (
    <div
      data-model-picker-footer
      className="flex min-h-11 flex-wrap items-center gap-2 border-t border-border/70 bg-muted/40 px-3 py-2"
    >
      {fast ? (
        <SpeedControl fastMode={fast.on} hint={fast.hint} onChange={fast.onChange} />
      ) : null}
      {permission ? (
        <PermissionModeMenu value={permission.value} onChange={permission.onChange} />
      ) : null}
    </div>
  );
}
