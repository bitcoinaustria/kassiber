import i18n from "@/i18n";
import { showFeedback } from "@/lib/feedback";
import { safeMessage } from "@/lib/notificationConditions";
import {
  conditionKey,
  useNotificationStore,
  type NotificationDetail,
  type NotificationScope,
} from "@/store/notifications";

/**
 * Notifications about one connection (a wallet, exchange account, or node)
 * of a book: its sync as a running job, the outcome as a Recent result, and
 * a failure as a condition whose action opens that connection.
 *
 * Conditions are keyed by the connection's id within the book, so one
 * connection's success never clears another's failure; a later successful
 * sync of the same connection resolves them.
 */

/** The scope of everything about one connection of `book`. */
export function connectionScope(
  book: string | null,
  connectionId: string,
): NotificationScope {
  return { book, source: `connection:${connectionId}` };
}

function jobId(connectionId: string): string {
  return `connection-sync:${connectionId}`;
}

export interface ConnectionRef {
  /** `bookIdentityKey` when the sync started. */
  book: string | null;
  connectionId: string;
  /** The user's label; only Normal and Hide amounts show it. */
  label?: string;
}

/** A sync of one connection is running; it shows under Running. */
export function startConnectionSync(
  ref: ConnectionRef,
  options: { forceFull?: boolean } = {},
): void {
  useNotificationStore.getState().startJob({
    id: jobId(ref.connectionId),
    code: "connection-sync",
    scope: connectionScope(ref.book, ref.connectionId),
    params: { forceFull: Boolean(options.forceFull) },
    progress: { indeterminate: true, label: ref.label },
  });
}

export function updateConnectionSync(
  connectionId: string,
  progress: { value?: number; indeterminate?: boolean; label?: string },
): void {
  useNotificationStore.getState().updateJob(jobId(connectionId), progress);
}

export function endConnectionSync(connectionId: string): void {
  useNotificationStore.getState().endJob(jobId(connectionId));
}

/**
 * A finished sync: what it proves resolves this connection's conditions,
 * and the outcome goes to Recent with a toast.
 */
export function recordConnectionSynced(
  ref: ConnectionRef,
  outcome: { forceFull?: boolean; imported?: number } = {},
): void {
  const scope = connectionScope(ref.book, ref.connectionId);
  const store = useNotificationStore.getState();
  store.resolveCondition(conditionKey("connection.sync-failed", scope));
  store.resolveCondition(conditionKey("btcpay.provenance-incomplete", scope));
  store.addResult({
    code: "connection.synced",
    scope,
    severity: "info",
    params: {
      forceFull: Boolean(outcome.forceFull),
      ...(outcome.imported !== undefined ? { transactions: outcome.imported } : {}),
    },
    action: { type: "open-connection", connectionId: ref.connectionId },
    details: ref.label ? [{ label: ref.label }] : undefined,
  });
  const t = i18n.getFixedT(null, "chrome");
  showFeedback({
    tone: "success",
    title: outcome.forceFull
      ? t("notifications.result.connectionSynced.rescanTitle")
      : t("notifications.result.connectionSynced.title"),
    description: ref.label,
    descriptionPrivacy: "free-text",
    key: jobId(ref.connectionId),
  });
}

/**
 * A sync that failed: a condition until a later sync of this connection
 * succeeds. `message` is operational text from the daemon; it is scrubbed
 * again and only shown behind Show details.
 */
export function recordConnectionSyncFailed(
  ref: ConnectionRef,
  message: string | undefined,
  extra: NotificationDetail[] = [],
): void {
  useNotificationStore.getState().raiseCondition({
    code: "connection.sync-failed",
    scope: connectionScope(ref.book, ref.connectionId),
    severity: "error",
    priority: "attention",
    action: { type: "open-connection", connectionId: ref.connectionId },
    details: [
      { label: ref.label, message: safeMessage(message), phraseKey: "failed" },
      ...extra.map((detail) => ({ ...detail, message: safeMessage(detail.message) })),
    ],
  });
  const t = i18n.getFixedT(null, "chrome");
  showFeedback({
    tone: "error",
    title: t("notifications.condition.connectionSyncFailed.title", { count: 1 }),
    description: ref.label,
    descriptionPrivacy: "free-text",
    key: jobId(ref.connectionId),
  });
}

/**
 * BTCPay saved the connection but could not load invoices for some stores:
 * payments lack their invoice details until a sync of that connection loads
 * them.
 */
export function recordBtcpayProvenanceIncomplete(
  ref: ConnectionRef,
  message: string | undefined,
): void {
  useNotificationStore.getState().raiseCondition({
    code: "btcpay.provenance-incomplete",
    scope: connectionScope(ref.book, ref.connectionId),
    severity: "warning",
    priority: "attention",
    action: { type: "open-connection", connectionId: ref.connectionId },
    details: [{ label: ref.label, message: safeMessage(message) }],
  });
}
