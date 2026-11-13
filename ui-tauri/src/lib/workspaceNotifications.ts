import i18n from "@/i18n";
import { showFeedback } from "@/lib/feedback";
import { safeMessage } from "@/lib/notificationConditions";
import {
  conditionKey,
  useNotificationStore,
  type JobProgress,
  type NotificationScope,
} from "@/store/notifications";

/**
 * Refreshing every book of a workspace from its overview of all books. The
 * outcome concerns several books, not the open one, so it belongs to no
 * book (it shows whichever is open) and is keyed by the workspace. Its
 * action opens that overview, where each book's state is listed.
 */
const JOB_ID = "workspace-refresh";

function scopeFor(workspaceId: string): NotificationScope {
  return { book: null, source: `workspace:${workspaceId}` };
}

export interface WorkspaceRefreshSummary {
  books: number;
  synced_books: number;
  errors: number;
  reports_blocked: number;
}

export function startWorkspaceRefresh(workspaceId: string): void {
  useNotificationStore.getState().startJob({
    id: JOB_ID,
    code: "workspace-refresh",
    scope: scopeFor(workspaceId),
    progress: { indeterminate: true },
  });
}

export function updateWorkspaceRefresh(progress: JobProgress): void {
  useNotificationStore.getState().updateJob(JOB_ID, progress);
}

export function endWorkspaceRefresh(): void {
  useNotificationStore.getState().endJob(JOB_ID);
}

export function recordWorkspaceRefreshed(
  workspaceId: string,
  summary: WorkspaceRefreshSummary | null,
): void {
  const scope = scopeFor(workspaceId);
  const store = useNotificationStore.getState();
  const action = { type: "open-workspace", workspaceId } as const;
  store.resolveCondition(conditionKey("workspace.refresh-failed", scope));
  const problems =
    summary !== null && (summary.errors > 0 || summary.reports_blocked > 0);
  if (problems) {
    store.raiseCondition({
      code: "workspace.books-need-attention",
      scope,
      severity: "warning",
      priority: summary.reports_blocked > 0 ? "blocking" : "attention",
      params: {
        count: Math.max(summary.errors, summary.reports_blocked),
        blocking: summary.reports_blocked > 0,
      },
      action,
    });
  } else if (summary !== null) {
    store.resolveCondition(conditionKey("workspace.books-need-attention", scope));
  }
  store.addResult({
    code: "workspace.refreshed",
    scope,
    severity: problems ? "warning" : "info",
    params: summary ? { count: summary.synced_books, entries: summary.books } : {},
    action,
  });
  const t = i18n.getFixedT(null, "chrome");
  showFeedback({
    tone: problems ? "warning" : "success",
    title: problems
      ? t("notifications.condition.workspaceBooks.title")
      : t("notifications.result.workspaceRefreshed.title"),
    key: JOB_ID,
  });
}

export function recordWorkspaceRefreshFailed(
  workspaceId: string,
  message: string | undefined,
): void {
  useNotificationStore.getState().raiseCondition({
    code: "workspace.refresh-failed",
    scope: scopeFor(workspaceId),
    severity: "error",
    priority: "attention",
    action: { type: "open-workspace", workspaceId },
    details: [{ message: safeMessage(message), phraseKey: "failed" }],
  });
  const t = i18n.getFixedT(null, "chrome");
  showFeedback({
    tone: "error",
    title: t("notifications.condition.workspaceRefreshFailed.title"),
    key: JOB_ID,
  });
}
