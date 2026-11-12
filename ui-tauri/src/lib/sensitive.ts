import * as React from "react";

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
 * "Hide amounts when Kassiber unlocks": raises the mode whenever the lock
 * state changes, including the first render at launch. A layout effect, so the
 * unlocked screen never paints with amounts visible.
 */
export function useUnlockPrivacy(locked: boolean) {
  React.useLayoutEffect(() => {
    useUiStore.getState().enforceUnlockPrivacy();
  }, [locked]);
}
