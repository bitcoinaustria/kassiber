import type { TFunction } from "i18next";

import type { PrivacyMode } from "@/store/ui";

/**
 * Free text (a sender's sentence, a backend error) may hold an amount that
 * nothing can tell apart from any other number, so Hide amounts treats any
 * digit as one.
 */
export function mayHoldAmount(text: string): boolean {
  return /\d/.test(text);
}

/**
 * Free text from a sender or server, as far as the privacy mode lets it
 * through: whole in Normal, withheld under Hide amounts if it contains a
 * digit, and dropped under Hide all.
 */
export function projectFreeText(
  text: string | undefined,
  mode: PrivacyMode,
  t: TFunction<"chrome">,
): string | undefined {
  const trimmed = text?.trim();
  if (!trimmed) return undefined;
  if (mode === "hideAll") return undefined;
  if (mode === "hideAmounts" && mayHoldAmount(trimmed)) {
    return t("notifications.withheld");
  }
  return trimmed;
}
