import {
  NOTIFICATION_OPERATION_KINDS,
  type NotificationActionSpec,
  type NotificationOperation,
  type NotificationScope,
} from "@/store/notifications";
import {
  NOTIFICATION_COMMANDS,
  type NotificationAction,
  type NotificationCommand,
} from "@/store/ui";

import { APP_ROUTE_PATHS, type AppRoutePath } from "./menuIntent";
import {
  settingsSearchEntry,
  type SettingsAnchorId,
} from "./settings/settingsSearch";
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

/** A structured entry's button, checked and bound to the open book. */
export type ResolvedEntryAction =
  | {
      kind: "navigate";
      to: NotificationTarget;
      stale: boolean;
      /** A settings row to reveal once the section shows. */
      anchor?: SettingsAnchorId;
    }
  | { kind: "open-connection"; connectionId: string }
  | { kind: "open-transaction"; transactionId: string }
  | { kind: "operation"; operation: NotificationOperation };

/** Record ids come from the daemon; anything odd shows no button. */
const MAX_RECORD_ID_CHARS = 256;
function validRecordId(value: unknown): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed && trimmed.length <= MAX_RECORD_ID_CHARS ? trimmed : null;
}

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

/** An anchor counts only on the settings route of its own section. */
function settingsAnchorFor(
  to: NotificationTarget,
  anchor: string | undefined,
): SettingsAnchorId | undefined {
  if (!anchor) return undefined;
  const entry = settingsSearchEntry(anchor);
  return entry && SETTINGS_SECTION_ROUTE[entry.section] === to
    ? entry.anchor
    : undefined;
}

/**
 * Resolve a structured action against the book open right now. Unknown
 * routes, anchors, records, and operations fail closed (no button). A
 * record or operation whose book is no longer open never opens or replays:
 * it becomes navigation to its list or review route, marked stale, so the
 * user decides again in the right place.
 */
export function resolveEntryAction(
  action: NotificationActionSpec | undefined,
  scope: NotificationScope,
  currentBook: string | null,
  developerToolsEnabled: boolean,
): ResolvedEntryAction | null {
  if (!action || typeof action !== "object") return null;
  const stale = scope.book !== currentBook;
  switch (action.type) {
    case "navigate": {
      const to = navigationTarget(action.to, developerToolsEnabled);
      if (!to) return null;
      const anchor = settingsAnchorFor(to, action.anchor);
      return anchor ? { kind: "navigate", to, stale, anchor } : { kind: "navigate", to, stale };
    }
    case "open-connection": {
      const connectionId = validRecordId(action.connectionId);
      if (!connectionId) return null;
      return stale
        ? { kind: "navigate", to: "/connections", stale }
        : { kind: "open-connection", connectionId };
    }
    case "open-transaction": {
      const transactionId = validRecordId(action.transactionId);
      if (!transactionId) return null;
      return stale
        ? { kind: "navigate", to: "/transactions", stale }
        : { kind: "open-transaction", transactionId };
    }
    case "operation": {
      const operation = validOperation(action.operation);
      const review = navigationTarget(action.review, developerToolsEnabled);
      if (!operation || !review) return null;
      if (stale) return { kind: "navigate", to: review, stale: true };
      return { kind: "operation", operation };
    }
    default:
      return null;
  }
}
