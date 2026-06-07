import type { AppNotification } from "@/store/ui";

function compactProgressTitle(title: string) {
  return title
    .replace(/\s+(started|running)$/i, "")
    .replace(/^BTC price refresh$/i, "BTC price")
    .replace(/^Journal processing$/i, "Journals")
    .trim();
}

export function routeProgressLabelFromNotifications(
  notifications: readonly AppNotification[],
) {
  const notification = notifications.find((item) => item.progress);
  if (!notification?.progress) return null;
  const title = compactProgressTitle(notification.title);
  const label = notification.progress.label?.trim();
  if (!label) return title || null;
  if (!title) return label;
  if (title.toLowerCase() === label.toLowerCase()) return title;
  return `${title}: ${label}`;
}
