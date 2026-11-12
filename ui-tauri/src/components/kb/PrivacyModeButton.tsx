import { Eye, EyeClosed, EyeOff, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import { PRIVACY_MODES, useUiStore, type PrivacyMode } from "@/store/ui";

const ICON: Record<PrivacyMode, LucideIcon> = {
  normal: Eye,
  hideAmounts: EyeOff,
  hideAll: EyeClosed,
};

const LABEL = {
  normal: "sensitive.modes.normal",
  hideAmounts: "sensitive.modes.hideAmounts",
  hideAll: "sensitive.modes.hideAll",
} as const satisfies Record<PrivacyMode, string>;

const DESCRIPTION = {
  normal: "sensitive.descriptions.normal",
  hideAmounts: "sensitive.descriptions.hideAmounts",
  hideAll: "sensitive.descriptions.hideAll",
} as const satisfies Record<PrivacyMode, string>;

/**
 * Title-bar privacy menu: pick Show all / Hide amounts / Hide all directly,
 * and whether every unlock starts hidden. ⌘⇧H (the native menu item) still
 * steps through the modes without opening the menu.
 */
export function PrivacyModeButton({ className }: { className?: string }) {
  const { t } = useTranslation("chrome");
  const privacyMode = useUiStore((s) => s.privacyMode);
  const setPrivacyMode = useUiStore((s) => s.setPrivacyMode);
  const hideAmountsOnUnlock = useUiStore((s) => s.hideAmountsOnUnlock);
  const setHideAmountsOnUnlock = useUiStore((s) => s.setHideAmountsOnUnlock);
  const Icon = ICON[privacyMode];
  const label = t("sensitive.trigger", { current: t(LABEL[privacyMode]) });
  const active = privacyMode !== "normal";
  return (
    <DropdownMenu modal={false}>
      <DropdownMenuTrigger asChild>
        <Button
          variant="ghost"
          size="icon"
          className={cn(
            className,
            active && "bg-sidebar-row-active [&>svg]:text-sidebar-foreground",
          )}
          aria-label={label}
          title={label}
          data-privacy-mode={privacyMode}
        >
          <Icon className="size-4" aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-72">
        <DropdownMenuLabel className="flex items-center">
          {t("sensitive.menuTitle")}
          <DropdownMenuShortcut>⌘⇧H</DropdownMenuShortcut>
        </DropdownMenuLabel>
        <DropdownMenuRadioGroup
          value={privacyMode}
          onValueChange={(next) => setPrivacyMode(next as PrivacyMode)}
        >
          {PRIVACY_MODES.map((mode) => (
            <DropdownMenuRadioItem key={mode} value={mode}>
              <span className="flex flex-col gap-0.5">
                <span>{t(LABEL[mode])}</span>
                <span className="text-xs text-muted-foreground">
                  {t(DESCRIPTION[mode])}
                </span>
              </span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
        <DropdownMenuSeparator />
        <DropdownMenuCheckboxItem
          checked={hideAmountsOnUnlock}
          onCheckedChange={(checked) => setHideAmountsOnUnlock(checked === true)}
          onSelect={(event) => event.preventDefault()}
        >
          {t("sensitive.hideOnUnlock")}
        </DropdownMenuCheckboxItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
