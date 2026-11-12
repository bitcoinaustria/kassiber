import {
  NOTIFICATION_COMMANDS,
  type NotificationAction,
  type NotificationCommand,
} from "@/store/ui";

import { APP_ROUTE_PATHS, type AppRoutePath } from "./menuIntent";
import {
  SETTINGS_SECTION_ROUTE,
  type SettingsSectionRoutePath,
} from "./settingsSections";

const SETTINGS_ROUTE_PATHS: readonly string[] = Object.values(
  SETTINGS_SECTION_ROUTE,
);

// Stored targets come back from `kb.ui`, so only known routes pass.
function asRoutePath(
  value: string | undefined,
): AppRoutePath | SettingsSectionRoutePath | undefined {
  if (!value) return undefined;
  if ((APP_ROUTE_PATHS as readonly string[]).includes(value)) {
    return value as AppRoutePath;
  }
  return SETTINGS_ROUTE_PATHS.includes(value)
    ? (value as SettingsSectionRoutePath)
    : undefined;
}

// Map a notification title to the screen that explains it.
export function notificationRouteFor(title: string): AppRoutePath | undefined {
  const normalized = title.toLowerCase();
  // Failures / "needs attention" go to Logs, which captures freshness job
  // errors (freshness._mark_error logs to the RAM ring). Checked FIRST so a
  // title like "Book refresh needs attention" isn't captured by the generic
  // "book"/"sync" keywords below and sent to an unrelated (empty) screen.
  if (
    normalized.includes("needs attention") ||
    normalized.includes("failed") ||
    normalized.includes("error") ||
    normalized.includes("daemon")
  ) {
    return "/logs";
  }
  if (normalized.includes("journal")) return "/journals";
  if (
    normalized.includes("quarantine") ||
    normalized.includes("quarantäne") ||
    normalized.includes("quarantaene")
  ) {
    return "/review";
  }
  if (normalized.includes("sync") || normalized.includes("wallet")) {
    return "/connections";
  }
  if (normalized.includes("report") || normalized.includes("export")) {
    return "/reports";
  }
  if (normalized.includes("book") || normalized.includes("books")) {
    return "/books";
  }
  if (normalized.includes("transaction")) return "/transactions";
  return undefined;
}

/**
 * A route the notification bell may open. Wider than `AppRoutePath` because the
 * developer-tools fallback needs a settings *section*: plain `/settings`
 * resolves to Appearance, which explains nothing about a failed sync.
 */
export type NotificationTarget = AppRoutePath | SettingsSectionRoutePath;

// Resolve the click target for a header notification, accounting for the fact
// that /logs is developer-tools-gated: its route guard bounces to /overview
// when developer tools are off. A failure notification that routed straight to
// /logs would therefore dead-end. Send those users to Settings -> Developer
// tools instead, where the switch is — for both the title router's /logs result
// and the error-tone fallback. Mirrors the search guard in appSearch.ts.
export function notificationTarget(
  title: string,
  tone: string | undefined,
  developerToolsEnabled: boolean,
  explicitTarget?: string,
): NotificationTarget | undefined {
  const logsOrSettings: NotificationTarget = developerToolsEnabled
    ? "/logs"
    : "/settings/developer";
  // An explicit, language-independent target set by the notification's producer
  // takes precedence over title keyword-matching (which only works in English).
  // It still flows through the /logs developer-tools guard below.
  const target = asRoutePath(explicitTarget) ?? notificationRouteFor(title);
  if (target === "/logs") return logsOrSettings;
  if (target) return target;
  return tone === "error" ? logsOrSettings : undefined;
}

/** A notification button, checked against the routes and commands it may use. */
export type ResolvedNotificationAction =
  | { label: string; kind: "navigate"; to: NotificationTarget }
  | { label: string; kind: "command"; command: NotificationCommand };

/**
 * Resolve a stored action into something the bell can run, or null. Persisted
 * state is not trusted to name a route or command: anything outside the known
 * lists is dropped, and a `/logs` target takes the same developer-tools
 * detour as a notification's own click target.
 */
export function resolveNotificationAction(
  action: NotificationAction | undefined,
  developerToolsEnabled: boolean,
): ResolvedNotificationAction | null {
  const label = action?.label?.trim();
  if (!action || !label) return null;
  if (
    action.command &&
    (NOTIFICATION_COMMANDS as readonly string[]).includes(action.command)
  ) {
    return { label, kind: "command", command: action.command };
  }
  const target = asRoutePath(action.target);
  if (!target) return null;
  if (target === "/logs") {
    return {
      label,
      kind: "navigate",
      to: developerToolsEnabled ? "/logs" : "/settings/developer",
    };
  }
  return { label, kind: "navigate", to: target };
}
