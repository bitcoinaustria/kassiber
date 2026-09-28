import i18n from "@/i18n";
import {
  describeJournalStep,
  journalStepNeedsAttention,
  type JournalStepSummary,
} from "@/lib/syncResults";
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

export function notifyJournalStepAfterImport(kind: string, data: unknown): void {
  if (!IMPORT_KINDS_WITH_JOURNAL_STEP.has(kind)) return;
  const journals = journalStepFromPayload(data);
  if (!journalStepNeedsAttention(journals)) return;
  useUiStore.getState().addNotification({
    title: i18n.t("connections:journalStep.attentionTitle"),
    body: describeJournalStep(journals) ?? "",
    tone: "warning",
    dedupeKey: "journal-step",
    target: "/journals",
  });
}
