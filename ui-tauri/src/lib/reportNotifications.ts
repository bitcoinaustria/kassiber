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
const JOB_ID = "report-export";

export function reportExportJob(book: string | null) {
  const scope: NotificationScope = { book };
  return {
    scope,
    start: () =>
      useNotificationStore.getState().startJob({
        id: JOB_ID,
        code: "report-export",
        scope,
        progress: { indeterminate: true },
      }),
    end: () => useNotificationStore.getState().endJob(JOB_ID),
  };
}

export function recordReportExported(
  scope: NotificationScope,
  outcome: {
    flagged: boolean;
    warnings: number;
    /** The saved file's name: user data, hidden under Hide all. */
    file: string;
    /** What it holds (pages, rows, files), as the Reports panel says it. */
    detail: string;
  },
): void {
  useNotificationStore.getState().addResult({
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
