/**
 * Provider marks and privacy-posture badges shared by the picker trigger,
 * provider rail and model rows. The posture badge is what tells the user
 * whether a prompt is about to leave the device (docs/reference/ai.md), so it
 * renders wherever a model can be chosen.
 */

import { useTranslation } from "react-i18next";

import type { AiProviderKind, AiProviderRow } from "@/lib/aiCapabilities";
import { cn } from "@/lib/utils";
import {
  PROVIDER_BRAND_ICON_BY_RUNTIME,
  type ProviderBrandIcon,
} from "./providerBrandIcons";
import {
  providerDisplayName,
  providerIconKey,
  providerInitials,
} from "./providerIdentity";

const BRAND_ICONS: Readonly<Record<string, ProviderBrandIcon | undefined>> =
  PROVIDER_BRAND_ICON_BY_RUNTIME;

const POSTURE_TONE: Record<AiProviderKind, string> = {
  local: "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300",
  remote: "bg-amber-500/15 text-amber-700 dark:text-amber-300",
  tee: "bg-sky-500/15 text-sky-700 dark:text-sky-300",
};

const POSTURE_DOT: Record<AiProviderKind, string> = {
  local: "bg-emerald-500",
  remote: "bg-amber-500",
  tee: "bg-sky-500",
};

export function ProviderGlyph({
  provider,
  className,
  iconClassName,
  dotPosture,
  dotClassName,
}: {
  provider: AiProviderRow;
  className?: string;
  iconClassName?: string;
  /** Overlay a posture dot (rail buttons) in this posture's colour. */
  dotPosture?: AiProviderKind;
  dotClassName?: string;
}) {
  const iconKey = providerIconKey(provider);
  const Icon = iconKey ? BRAND_ICONS[iconKey] : undefined;
  return (
    <span
      className={cn(
        "relative inline-flex size-4 shrink-0 items-center justify-center",
        className,
      )}
      aria-hidden="true"
    >
      {Icon ? (
        <Icon className={cn("size-full", iconClassName)} />
      ) : (
        <span
          className={cn(
            "text-3xs font-semibold leading-none tracking-tight text-foreground/80",
            iconClassName,
          )}
        >
          {providerInitials(providerDisplayName(provider))}
        </span>
      )}
      {dotPosture ? (
        <span
          className={cn(
            "pointer-events-none absolute -top-0.5 -right-0.5 size-1.5 rounded-full ring-2 ring-popover",
            POSTURE_DOT[dotPosture],
            dotClassName,
          )}
        />
      ) : null}
    </span>
  );
}

export function PostureBadge({
  posture,
  title,
  className,
}: {
  posture: AiProviderKind;
  title?: string;
  className?: string;
}) {
  const { t } = useTranslation("assistant");
  return (
    <span
      className={cn(
        "inline-flex shrink-0 items-center rounded-full px-1.5 py-0.5 text-3xs font-medium uppercase leading-none tracking-wide",
        POSTURE_TONE[posture],
        className,
      )}
      title={title ?? t(`modelPicker.postureHint.${posture}`)}
      data-posture={posture}
    >
      {t(`modelPicker.posture.${posture}`)}
    </span>
  );
}
