import i18n from "@/i18n";
import { showFeedback } from "@/lib/feedback";
import { safeMessage } from "@/lib/notificationConditions";
import {
  conditionKey,
  useNotificationStore,
  type NotificationScope,
} from "@/store/notifications";

/**
 * Rebuilding a book's prices from Settings -> Market data. It runs on after
 * Settings closes, so it is a job while it runs and a Recent result when it
 * finishes. A rebuild that failed is a condition opening the rebuild row;
 * one whose journal step failed raises the journal-processing condition,
 * since reprocessing journals is what applies the new prices.
 */
/** Toasts about rebuilds replace one another. */
const FEEDBACK_KEY = "pricing-rebuild";
/**
 * Each rebuild is its own job: one started after leaving and re-entering
 * Market data while another still runs must not replace or end that one.
 */
let rebuildSequence = 0;
const REBUILD_ACTION = {
  type: "navigate",
  to: "/settings/market",
  anchor: "rebuildPricing",
} as const;

/**
 * `book` is the open book bound at the click (`activeBookScope`); null while
 * it is unknown, in which case the job still shows but its outcome files
 * nothing in the center.
 */
export function pricingRebuildJob(book: NotificationScope | null) {
  const scope: NotificationScope | null = book;
  const id = `pricing-rebuild:${++rebuildSequence}`;
  return {
    scope,
    start: () =>
      useNotificationStore.getState().startJob({
        id,
        code: "pricing-rebuild",
        scope: scope ?? { book: null },
        progress: { indeterminate: true },
      }),
    end: () => useNotificationStore.getState().endJob(id),
  };
}

export function recordPricingRebuilt(
  scope: NotificationScope | null,
  outcome: { summary: string; journalBlocker?: string },
): void {
  const store = useNotificationStore.getState();
  if (scope) store.resolveCondition(conditionKey("pricing.rebuild-failed", scope));
  // Its Retry processes journals with `expected_scope`, so it needs the
  // book's daemon ids; without them nothing is filed that could run.
  if (scope?.expected && outcome.journalBlocker) {
    store.raiseCondition({
      code: "journals.process-failed",
      scope,
      severity: "error",
      priority: "attention",
      action: {
        type: "operation",
        operation: { kind: "process-journals", autoPair: true },
        review: "/journals",
      },
      details: [{ message: safeMessage(outcome.journalBlocker) }],
    });
  }
  if (scope) store.addResult({
    code: "pricing.rebuilt",
    scope,
    severity: outcome.journalBlocker ? "warning" : "info",
    action: REBUILD_ACTION,
    details: [{ message: outcome.summary }],
  });
  const t = i18n.getFixedT(null, "chrome");
  showFeedback({
    tone: outcome.journalBlocker ? "warning" : "success",
    title: t("notifications.result.pricingRebuilt.title"),
    description: outcome.summary,
    descriptionPrivacy: "free-text",
    key: FEEDBACK_KEY,
  });
}

export function recordPricingRebuildFailed(
  scope: NotificationScope | null,
  message: string,
): void {
  if (scope) useNotificationStore.getState().raiseCondition({
    code: "pricing.rebuild-failed",
    scope,
    severity: "error",
    priority: "attention",
    action: REBUILD_ACTION,
    details: [{ message: safeMessage(message) }],
  });
  const t = i18n.getFixedT(null, "chrome");
  showFeedback({
    tone: "error",
    title: t("notifications.condition.pricingRebuildFailed.title"),
    key: FEEDBACK_KEY,
  });
}
