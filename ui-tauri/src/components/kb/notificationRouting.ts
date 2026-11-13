import type { ActiveBook, DaemonBookScope } from "@/lib/activeBook";
import {
  NOTIFICATION_OPERATION_KINDS,
  type NotificationActionSpec,
  type NotificationOperation,
  type NotificationScope,
} from "@/store/notifications";
import type { NotificationAction } from "@/store/ui";

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

// Map a legacy notification's title to the screen that explains it. Only
// senders still on `addNotification` reach this; structured entries name
// their destination and never route from (translated) prose.
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

/** A legacy notification button: navigation to a known route, nothing else. */
export type ResolvedNotificationAction = {
  label: string;
  kind: "navigate";
  to: NotificationTarget;
};

/**
 * Resolve a legacy action into a route the bell may open, or null. Only
 * known routes pass, and a `/logs` target takes the same developer-tools
 * detour as a notification's own click target. A legacy entry has no book
 * scope or original options, so anything that would mutate (a `command`
 * from an older sender) is refused rather than replayed against the open
 * book.
 */
export function resolveNotificationAction(
  action: NotificationAction | undefined,
  developerToolsEnabled: boolean,
): ResolvedNotificationAction | null {
  const label = action?.label?.trim();
  if (!action || !label) return null;
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

/** A structured entry's button, checked and bound to the open book. */
export type ResolvedEntryAction =
  | { kind: "navigate"; to: NotificationTarget; stale: boolean }
  | {
      kind: "operation";
      operation: NotificationOperation;
      /** Sent as `expected_scope`: the daemon refuses it in another book. */
      expectedScope: DaemonBookScope;
    };

function validOperation(
  operation: NotificationOperation | undefined,
): NotificationOperation | null {
  if (!operation || typeof operation !== "object") return null;
  if (
    !(NOTIFICATION_OPERATION_KINDS as readonly string[]).includes(operation.kind)
  ) {
    return null;
  }
  if (operation.kind === "refresh-book") {
    return typeof operation.forceFull === "boolean"
      ? { kind: "refresh-book", forceFull: operation.forceFull }
      : null;
  }
  return typeof operation.autoPair === "boolean"
    ? { kind: "process-journals", autoPair: operation.autoPair }
    : null;
}

function navigationTarget(
  value: string,
  developerToolsEnabled: boolean,
): NotificationTarget | null {
  const target = asRoutePath(value);
  if (!target) return null;
  if (target === "/logs") {
    return developerToolsEnabled ? "/logs" : "/settings/developer";
  }
  return target;
}

/**
 * Resolve a structured action against the book open right now (`current`,
 * null while unknown or while a switch is in flight). Unknown routes and
 * operations fail closed (no button). An operation runs only for the book it
 * came from, and only with that book's daemon ids to send as
 * `expected_scope`; otherwise it becomes navigation to its review route,
 * marked stale, so the user decides again in the right place.
 */
export function resolveEntryAction(
  action: NotificationActionSpec | undefined,
  scope: NotificationScope,
  current: ActiveBook | null,
  developerToolsEnabled: boolean,
): ResolvedEntryAction | null {
  if (!action || typeof action !== "object") return null;
  const sameBook = current !== null && scope.book === current.key;
  if (action.type === "navigate") {
    const to = navigationTarget(action.to, developerToolsEnabled);
    return to
      ? { kind: "navigate", to, stale: scope.book !== null && !sameBook }
      : null;
  }
  if (action.type === "operation") {
    const operation = validOperation(action.operation);
    const review = navigationTarget(action.review, developerToolsEnabled);
    if (!operation || !review) return null;
    if (
      current === null ||
      !sameBook ||
      !scope.expected ||
      scope.expected.workspace_id !== current.expected.workspace_id ||
      scope.expected.profile_id !== current.expected.profile_id
    ) {
      return { kind: "navigate", to: review, stale: true };
    }
    return { kind: "operation", operation, expectedScope: scope.expected };
  }
  return null;
}
