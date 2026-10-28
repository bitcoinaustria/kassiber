/**
 * The chat's permission mode in the composer toolbar: icon and label on the
 * trigger, each option with its one-line description, and the current
 * description as the tooltip. Full access marks its trigger in a
 * warning tone, since it also runs network actions without asking.
 *
 * Modelled on T3 Code's runtime-mode select (MIT, Copyright (c) 2026 T3
 * Tools Inc.): `apps/web/src/components/chat/ComposerFooterControls.tsx` and
 * `runtimeModeConfig.ts`. Reimplemented with Kassiber's composer controls and
 * menu primitives.
 */

import type { LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

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

import { ComposerControl, ComposerControlChevron } from "./ComposerControl";
import { PERMISSION_MODE_OPTIONS, permissionModeOption } from "./permissionMode";

interface MenuOption<T extends string> {
  value: T;
  icon: LucideIcon;
  label: string;
  description: string;
  warning?: boolean;
}

interface PickerProps<T extends string> {
  value: T;
  onChange: (value: T) => void;
  onOpenChange?: (open: boolean) => void;
  /** Classes for the trigger's label, e.g. to hide it where space is short. */
  labelClassName?: string;
}

const WARNING_TONE =
  "text-amber-700 hover:text-amber-800 dark:text-amber-400 dark:hover:text-amber-300";

function ComposerOptionMenu<T extends string>({
  name,
  title,
  value,
  options,
  isValue,
  onChange,
  onOpenChange,
  labelClassName,
}: PickerProps<T> & {
  /** Stable id for tests and styling (`data-composer-option`). */
  name: string;
  title: string;
  options: readonly MenuOption<T>[];
  isValue: (value: unknown) => value is T;
}) {
  const current = options.find((option) => option.value === value) ?? options[0]!;
  const CurrentIcon = current.icon;
  return (
    <DropdownMenu onOpenChange={onOpenChange}>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <ComposerControl
              aria-label={`${title}: ${current.label}`}
              data-composer-option={name}
              data-value={value}
              className={cn("shrink-0", current.warning && WARNING_TONE)}
            >
              <CurrentIcon className="size-4" aria-hidden="true" />
              <span className={cn("truncate", labelClassName)}>{current.label}</span>
              <ComposerControlChevron />
            </ComposerControl>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent side="top" sideOffset={6} className="max-w-64 text-pretty">
          {current.description}
        </TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="start" side="top" className="w-72">
        <DropdownMenuLabel className="text-xs font-medium text-muted-foreground">
          {title}
        </DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={value}
          onValueChange={(next) => {
            if (isValue(next)) onChange(next);
          }}
        >
          {options.map((option) => {
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
                        option.warning
                          ? "text-amber-600 dark:text-amber-400"
                          : "text-muted-foreground",
                      )}
                      aria-hidden="true"
                    />
                    {option.label}
                  </span>
                  <span className="text-xs leading-4 text-muted-foreground">
                    {option.description}
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

/** Approval / Auto / Full access for this chat. */
export function PermissionModePicker(props: PickerProps<AiPermissionMode>) {
  const { t } = useTranslation("assistant");
  // Resolved through the helper so an unknown value still shows a mode.
  const value = permissionModeOption(props.value).value;
  return (
    <ComposerOptionMenu
      {...props}
      value={value}
      name="permission-mode"
      title={t("composer.permission.label")}
      isValue={isAiPermissionMode}
      options={PERMISSION_MODE_OPTIONS.map((option) => ({
        value: option.value,
        icon: option.icon,
        label: t(option.labelKey),
        description: t(option.descriptionKey),
        // A quiet warning, as T3 Code marks its open lock.
        warning: option.value === "full_access",
      }))}
    />
  );
}
