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
const JOB_ID = "pricing-rebuild";
const REBUILD_ACTION = {
  type: "navigate",
  to: "/settings/market",
  anchor: "rebuildPricing",
} as const;

export function pricingRebuildJob(book: string | null) {
  const scope: NotificationScope = { book };
  return {
    scope,
    start: () =>
      useNotificationStore.getState().startJob({
        id: JOB_ID,
        code: "pricing-rebuild",
        scope,
        progress: { indeterminate: true },
      }),
    end: () => useNotificationStore.getState().endJob(JOB_ID),
  };
}

export function recordPricingRebuilt(
  scope: NotificationScope,
  outcome: { summary: string; journalBlocker?: string },
): void {
  const store = useNotificationStore.getState();
  store.resolveCondition(conditionKey("pricing.rebuild-failed", scope));
  if (outcome.journalBlocker) {
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
  store.addResult({
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
    key: JOB_ID,
  });
}

export function recordPricingRebuildFailed(
  scope: NotificationScope,
  message: string,
): void {
  useNotificationStore.getState().raiseCondition({
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
    key: JOB_ID,
  });
}
