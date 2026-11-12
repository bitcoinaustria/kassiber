import * as React from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import { useUiStore, type PrivacyMode } from "@/store/ui";

/**
 * What a hidden value renders instead of its digits. Fixed width, so the
 * mask reveals nothing about the magnitude either.
 */
export const SENSITIVE_MASK = "••••";

/**
 * The blur class for values that stay in the DOM while hidden (addresses,
 * dense table cells). Prefer `<Sensitive>` where the value can be replaced.
 */
export const sensitiveClass = (hidden: boolean) => (hidden ? "sensitive" : "");

/** What a value reveals: an amount, or a share of the whole. */
export type SensitiveKind = "amount" | "share";

export function isHidden(mode: PrivacyMode, kind: SensitiveKind = "amount") {
  return kind === "share" ? mode === "hideAll" : mode !== "normal";
}

/** Whether values of this kind are hidden under the current privacy mode. */
export function useSensitive(kind: SensitiveKind = "amount") {
  return useUiStore((s) => isHidden(s.privacyMode, kind));
}

/** Text form for attributes and SVG labels, where no element can be swapped. */
export function maskSensitive(text: string, hidden: boolean) {
  return hidden ? SENSITIVE_MASK : text;
}

/**
 * Renders its children, or the mask when the privacy mode hides this kind of
 * value. The real value is not rendered at all, so it is absent from the DOM,
 * the accessibility tree, and find-in-page.
 */
export function Sensitive({
  kind = "amount",
  hidden,
  className,
  children,
}: {
  kind?: SensitiveKind;
  /** Overrides the store, for callers that already hold the flag. */
  hidden?: boolean;
  className?: string;
  children: React.ReactNode;
}) {
  const fromStore = useSensitive(kind);
  const { t } = useTranslation("common");
  if (!(hidden ?? fromStore)) return <>{children}</>;
  return (
    <span
      className={cn("sensitive", className)}
      aria-label={kind === "share" ? t("privacy.hiddenShare") : t("privacy.hiddenAmount")}
      data-sensitive-mask=""
    >
      {SENSITIVE_MASK}
    </span>
  );
}

/**
 * "Hide amounts when Kassiber unlocks": raises the mode whenever the lock
 * state changes, including the first render at launch. A layout effect, so the
 * unlocked screen never paints with amounts visible.
 */
export function useUnlockPrivacy(locked: boolean) {
  React.useLayoutEffect(() => {
    useUiStore.getState().enforceUnlockPrivacy();
  }, [locked]);
}
