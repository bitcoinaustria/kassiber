function leadingProgressPhrase(text: string): string {
  return text
    .trim()
    .replace(/\s+/g, " ")
    .replace(/[.;]\s*$/, "")
    .split(/\s*(?:[;·])\s*/)[0]
    .trim()
    .toLocaleLowerCase();
}

export function shouldHideNotificationProgressLabel(
  body: string,
  progressLabel: string | null | undefined,
): boolean {
  if (!progressLabel?.trim()) return false;
  const bodyLead = leadingProgressPhrase(body);
  const labelLead = leadingProgressPhrase(progressLabel);
  return Boolean(bodyLead && labelLead && bodyLead === labelLead);
}

/**
 * Body length past which a collapsed entry clips to two lines. Measuring the
 * rendered height (as rotki does) would make the toggle depend on layout; a
 * length check behaves the same in the app and in tests.
 */
export const NOTIFICATION_BODY_COLLAPSE_CHARS = 120;

/** Whether an entry has more to show than its collapsed two lines. */
export function notificationIsExpandable(item: {
  body: string;
  details?: readonly string[];
}): boolean {
  if (item.details?.some((line) => line.trim())) return true;
  const body = item.body.trim();
  return body.length > NOTIFICATION_BODY_COLLAPSE_CHARS || body.includes("\n");
}

/** A progress value clamped to 0–100 for a bar width. */
export function notificationProgressValue(value: number | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, value));
}
