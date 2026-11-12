import { Eye, EyeClosed, EyeOff, type LucideIcon } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { nextPrivacyMode, useUiStore, type PrivacyMode } from "@/store/ui";

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

/** Title-bar control: Normal → Hide amounts → Hide all → Normal. */
export function PrivacyModeButton({ className }: { className?: string }) {
  const { t } = useTranslation("chrome");
  const privacyMode = useUiStore((s) => s.privacyMode);
  const cyclePrivacyMode = useUiStore((s) => s.cyclePrivacyMode);
  const Icon = ICON[privacyMode];
  const label = t("sensitive.cycle", {
    current: t(LABEL[privacyMode]),
    next: t(LABEL[nextPrivacyMode(privacyMode)]),
  });
  const active = privacyMode !== "normal";
  return (
    <Button
      variant="ghost"
      size="icon"
      className={cn(
        className,
        active && "bg-sidebar-row-active [&>svg]:text-sidebar-foreground",
      )}
      aria-label={label}
      aria-pressed={active}
      title={label}
      data-privacy-mode={privacyMode}
      onClick={cyclePrivacyMode}
    >
      <Icon className="size-4" aria-hidden="true" />
    </Button>
  );
}
