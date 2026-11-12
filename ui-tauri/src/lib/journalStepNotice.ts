import i18n from "@/i18n";
import { showFeedback } from "@/lib/feedback";
import { safeMessage } from "@/lib/notificationConditions";
import {
  describeJournalStep,
  journalStepNeedsAttention,
  type JournalStepSummary,
} from "@/lib/syncResults";
import { useNotificationStore } from "@/store/notifications";
import { bookIdentityKey, useUiStore } from "@/store/ui";

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
 * What a journal step that did not finish leaves behind for `book`:
 *
 * - failed or cancelled: the journal-processing failure condition, whose
 *   action processes journals again;
 * - deferred or disabled: the stale-journals condition, the same one the
 *   live Review badges raise, which the next successful badge read resolves;
 * - processed but automatic pairing skipped: a toast only. The journals are
 *   current, and any pairing left to decide shows in the Review condition.
 */
export function recordJournalStep(
  journals: JournalStepSummary | null | undefined,
  book: string | null,
): void {
  if (!journalStepNeedsAttention(journals)) return;
  const store = useNotificationStore.getState();
  const scope = { book };
  const operation = {
    type: "operation",
    operation: { kind: "process-journals", autoPair: true },
    review: "/journals",
  } as const;
  const status = journals?.status;
  if (status === "failed" || status === "cancelled") {
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
  if (status === "deferred" || status === "disabled") {
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

export function notifyJournalStepAfterImport(kind: string, data: unknown): void {
  if (!IMPORT_KINDS_WITH_JOURNAL_STEP.has(kind)) return;
  recordJournalStep(
    journalStepFromPayload(data),
    bookIdentityKey(useUiStore.getState().identity),
  );
}
