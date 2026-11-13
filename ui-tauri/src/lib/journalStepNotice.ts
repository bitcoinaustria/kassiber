import type { QueryClient } from "@tanstack/react-query";

import i18n from "@/i18n";
import { showFeedback } from "@/lib/feedback";
import { safeMessage } from "@/lib/notificationConditions";
import { activeBookScope } from "@/lib/notificationScope";
import {
  describeJournalStep,
  journalStepNeedsAttention,
  type JournalStepSummary,
} from "@/lib/syncResults";
import {
  useNotificationStore,
  type NotificationScope,
} from "@/store/notifications";
import { useUiStore } from "@/store/ui";

// Imports finish with the daemon's local journal step. Their dialogs report the
// import itself; this keeps a failed, deferred, or disabled journal update from
// passing silently whichever surface started the import.
const IMPORT_KINDS_WITH_JOURNAL_STEP = new Set([
  "ui.wallets.import_file",
  "ui.wallets.document_import.import",
  "ui.wallets.import_samourai",
]);

export function journalStepFromPayload(data: unknown): JournalStepSummary | null {
  if (!data || typeof data !== "object" || !("journals" in data)) return null;
  const journals = (data as { journals?: unknown }).journals;
  return journals && typeof journals === "object"
    ? (journals as JournalStepSummary)
    : null;
}

/**
 * What a journal step that did not finish leaves behind for the book it ran
 * in (`scope`, bound when the work started):
 *
 * - failed or cancelled: the journal-processing failure condition, whose
 *   action processes journals again;
 * - deferred or disabled: the stale-journals condition, the same one the
 *   live Review badges raise, which the next successful badge read resolves;
 * - processed but automatic pairing skipped: a toast only. The journals are
 *   current, and any pairing left to decide shows in the Review condition.
 *
 * The conditions' action processes journals with `expected_scope`, so they
 * are filed only with the book's daemon ids; without them (the open book
 * was unknown) only a toast says so.
 */
export function recordJournalStep(
  journals: JournalStepSummary | null | undefined,
  bound: NotificationScope | null,
): void {
  if (!journalStepNeedsAttention(journals)) return;
  const store = useNotificationStore.getState();
  const scope: NotificationScope | null =
    bound?.expected && bound.book !== null
      ? { book: bound.book, expected: bound.expected }
      : null;
  const operation = {
    type: "operation",
    operation: { kind: "process-journals", autoPair: true },
    review: "/journals",
  } as const;
  const status = journals?.status;
  if (scope && (status === "failed" || status === "cancelled")) {
    store.raiseCondition({
      code: "journals.process-failed",
      scope,
      severity: "error",
      priority: "attention",
      action: operation,
      details: [
        {
          message: safeMessage(journals?.error?.message ?? journals?.error?.code),
          phraseKey: status === "cancelled" ? "cancelled" : "failed",
        },
      ],
    });
    return;
  }
  if (scope && (status === "deferred" || status === "disabled")) {
    store.raiseCondition({
      code: "journals.stale",
      scope,
      severity: "warning",
      priority: "attention",
      action: operation,
    });
    return;
  }
  showFeedback({
    tone: "warning",
    title: i18n.t("connections:journalStep.attentionTitle"),
    description: describeJournalStep(journals) ?? undefined,
    descriptionPrivacy: "free-text",
    key: "journal-step",
  });
}

/**
 * After an import, for the book open when it started. A daemon session that
 * changed meanwhile proves nothing about the book open now, so nothing is
 * filed then.
 */
export function notifyJournalStepAfterImport(
  queryClient: QueryClient,
  startedSession: number,
  kind: string,
  data: unknown,
): void {
  if (!IMPORT_KINDS_WITH_JOURNAL_STEP.has(kind)) return;
  const sameSession = useUiStore.getState().daemonSession === startedSession;
  recordJournalStep(
    journalStepFromPayload(data),
    sameSession ? activeBookScope(queryClient) : null,
  );
}
