import {
  NOTIFICATION_OPERATION_KINDS,
  type NotificationActionSpec,
  type NotificationOperation,
  type NotificationScope,
} from "@/store/notifications";

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

// Only known routes pass: an action names its route as data.
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

/**
 * A route the notification bell may open. Wider than `AppRoutePath`
 * because an action can name a settings *section*: plain `/settings`
 * resolves to Appearance, which explains nothing about a failed sync.
 */
export type NotificationTarget = AppRoutePath | SettingsSectionRoutePath;

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
  | { kind: "open-workspace"; workspaceId: string }
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
    case "open-workspace": {
      // A workspace overview spans its books, so no book can make it stale.
      const workspaceId = validRecordId(action.workspaceId);
      return workspaceId ? { kind: "open-workspace", workspaceId } : null;
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
