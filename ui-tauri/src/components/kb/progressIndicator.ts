import type { AppNotification } from "@/store/ui";

export interface RouteProgressState {
  indeterminate: boolean;
  label: string;
  value?: number;
}

function compactProgressTitle(title: string) {
  return title
    .replace(/\s+(started|running)$/i, "")
    .replace(/^BTC price refresh$/i, "BTC price")
    .replace(/^Journal processing$/i, "Journals")
    .trim();
}

export function routeProgressFromNotifications(
  notifications: readonly AppNotification[],
): RouteProgressState | null {
  const notification = notifications.find((item) => item.progress);
  if (!notification?.progress) return null;
  const title = compactProgressTitle(notification.title);
  const label = notification.progress.label?.trim();
  const displayLabel = (() => {
    if (!label) return title || "In progress";
    if (!title) return label;
    if (title.toLowerCase() === label.toLowerCase()) return title;
    return `${title}: ${label}`;
  })();

  return {
    indeterminate: Boolean(notification.progress.indeterminate),
    label: displayLabel,
    value: notification.progress.value,
  };
}

export function routeProgressLabelFromNotifications(
  notifications: readonly AppNotification[],
) {
  return routeProgressFromNotifications(notifications)?.label ?? null;
}
