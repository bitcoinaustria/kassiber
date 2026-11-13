import i18n from "@/i18n";
import { showFeedback } from "@/lib/feedback";
import { useNotificationStore, type NotificationScope } from "@/store/notifications";

/**
 * A report export runs on after the user leaves Reports, so it is a job
 * while it runs and a Recent result when it finishes, opening Reports where
 * the saved file and any notes are listed. Failures are not conditions: the
 * files panel keeps the error, and a quarantine block is already the Review
 * condition.
 */
/**
 * Each export is its own job: one started after leaving and re-entering
 * Reports while another still runs must not replace that one's row, nor be
 * ended by it.
 */
let exportSequence = 0;

/**
 * `book` is the open book bound at the click (`activeBookScope`); null while
 * it is unknown, in which case the job still shows but its outcome files
 * nothing in the center.
 */
export function reportExportJob(book: NotificationScope | null) {
  const scope: NotificationScope | null = book;
  const id = `report-export:${++exportSequence}`;
  return {
    scope,
    start: () =>
      useNotificationStore.getState().startJob({
        id,
        code: "report-export",
        scope: scope ?? { book: null },
        progress: { indeterminate: true },
      }),
    end: () => useNotificationStore.getState().endJob(id),
  };
}

export function recordReportExported(
  scope: NotificationScope | null,
  outcome: {
    flagged: boolean;
    warnings: number;
    /** The saved file's name: user data, hidden under Hide all. */
    file: string;
    /** What it holds (pages, rows, files), as the Reports panel says it. */
    detail: string;
  },
): void {
  if (scope) useNotificationStore.getState().addResult({
    code: "report.exported",
    scope,
    severity: outcome.flagged ? "warning" : "info",
    params: { warnings: outcome.warnings },
    action: { type: "navigate", to: "/reports" },
    details: [{ label: outcome.file, message: outcome.detail }],
  });
  const t = i18n.getFixedT(null, "chrome");
  showFeedback({
    tone: outcome.flagged ? "warning" : "success",
    title: outcome.flagged
      ? t("notifications.result.reportExported.notesTitle")
      : t("notifications.result.reportExported.title"),
    description: outcome.file,
    descriptionPrivacy: "free-text",
    key: "report-export",
  });
}
