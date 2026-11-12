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

export type NotificationFilter = "all" | "needsAction";

/**
 * Whether a header notification asks the user to do something. Warnings and
 * errors do, as do items that carry an inline action (Process now). A click
 * target alone does not: nearly every notification resolves one from its
 * title, including finished syncs. A running job does not either, even when
 * its producer gives it a warning tone while it is in flight; it needs action
 * only once it settles into a warning or error.
 */
export function notificationNeedsAction(item: {
  tone: string;
  progress?: unknown;
  action?: string;
}): boolean {
  if (item.action) return true;
  if (item.progress) return false;
  return item.tone === "warning" || item.tone === "error";
}

export function filterNotifications<
  T extends { tone: string; progress?: unknown; action?: string },
>(items: readonly T[], filter: NotificationFilter): T[] {
  return filter === "needsAction"
    ? items.filter(notificationNeedsAction)
    : [...items];
}
