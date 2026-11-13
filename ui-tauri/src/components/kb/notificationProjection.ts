import type { TFunction } from "i18next";

import type { AppNotification, NotificationTone, PrivacyMode } from "@/store/ui";
import type {
  ConditionGroup,
  JobEntry,
  NotificationDetail,
  NotificationSeverity,
  ResultEntry,
} from "@/store/notifications";

import { mayHoldAmount, projectFreeText } from "./notificationPrivacy";

/**
 * What the center and the toasts may show of an entry in the current privacy
 * mode. Everything visible, including aria labels and titles, comes from
 * here, never from a stored sentence:
 *
 * - **Normal**: translated messages with user labels and bounded,
 *   credential-scrubbed operational detail.
 * - **Hide amounts**: as Normal, but free text that contains a digit is
 *   withheld, since it cannot be told apart from a financial value. Typed
 *   counts (sources, decisions, transactions) stay.
 * - **Hide all**: generic titles and bodies; no labels, details, or free
 *   text. Actions keep working.
 *
 * Locked shows nothing: the shell hides the bell and the toast layer while
 * the lock screen is up.
 */
export interface ProjectedEntry {
  title: string;
  body?: string;
  details: string[];
}

type ChromeT = TFunction<"chrome">;

function projectDetail(
  detail: NotificationDetail,
  mode: PrivacyMode,
  t: ChromeT,
): string | null {
  if (mode === "hideAll") return null;
  const phrase = detail.phraseKey
    ? t(`notifications.phrase.${detail.phraseKey}`)
    : undefined;
  const message = projectFreeText(detail.message, mode, t);
  const text = [phrase, message].filter(Boolean).join(" — ");
  if (detail.label && text) {
    return t("notifications.detailLine", { label: detail.label, message: text });
  }
  return detail.label ?? (text || null);
}

function projectDetails(
  details: readonly NotificationDetail[] | undefined,
  mode: PrivacyMode,
  t: ChromeT,
): string[] {
  return (details ?? [])
    .map((detail) => projectDetail(detail, mode, t))
    .filter((line): line is string => Boolean(line));
}

/** One attention row: what is wrong and what it means for reports. */
export function projectConditionGroup(
  group: ConditionGroup,
  mode: PrivacyMode,
  t: ChromeT,
): ProjectedEntry {
  const first = group.entries[0];
  const details = group.entries.flatMap((entry) =>
    projectDetails(entry.details, mode, t),
  );
  switch (group.code) {
    case "journals.stale":
      return {
        title: t("notifications.condition.journalsStale.title"),
        body: t("notifications.condition.journalsStale.body"),
        details: [],
      };
    case "review.pending": {
      const count = first.params.count ?? 0;
      return {
        title: t("notifications.condition.reviewPending.title"),
        body: first.params.blocking
          ? t("notifications.condition.reviewPending.blockingBody", { count })
          : t("notifications.condition.reviewPending.body", { count }),
        details: [],
      };
    }
    case "source.refresh-failed":
      return {
        title: t("notifications.condition.sourceFailed.title", {
          count: group.entries.length,
        }),
        body:
          group.priority === "blocking"
            ? t("notifications.condition.sourceFailed.blockingBody")
            : t("notifications.condition.sourceFailed.body"),
        details,
      };
    case "freshness.worker-error":
      return {
        title: t("notifications.condition.workerError.title"),
        body: t("notifications.condition.workerError.body"),
        details,
      };
    case "book-refresh.failed":
      return {
        title: first.params.forceFull
          ? t("notifications.condition.bookRefreshFailed.rescanTitle")
          : t("notifications.condition.bookRefreshFailed.title"),
        body: t("notifications.condition.bookRefreshFailed.body"),
        details,
      };
    case "book-refresh.backend-required":
      return {
        title: t("notifications.condition.backendRequired.title"),
        body: t("notifications.condition.backendRequired.body"),
        details,
      };
    case "journals.process-failed":
      return {
        title: t("notifications.condition.journalsFailed.title"),
        body: t("notifications.condition.journalsFailed.body"),
        details,
      };
    case "connection.sync-failed":
      return {
        title: t("notifications.condition.connectionSyncFailed.title", {
          count: group.entries.length,
        }),
        body: t("notifications.condition.connectionSyncFailed.body"),
        details,
      };
    case "btcpay.provenance-incomplete":
      return {
        title: t("notifications.condition.btcpayProvenance.title"),
        body: t("notifications.condition.btcpayProvenance.body"),
        details,
      };
    case "touch-id.off":
      return {
        title: t("notifications.condition.touchIdOff.title"),
        body: t("notifications.condition.touchIdOff.body"),
        details,
      };
    case "unlock.credential-remains":
      return {
        title: t("notifications.condition.unlockCredentialRemains.title"),
        body: t("notifications.condition.unlockCredentialRemains.body"),
        details,
      };
    case "backup.restore-failed":
      return {
        title: t("notifications.condition.backupRestoreFailed.title"),
        body: t("notifications.condition.backupRestoreFailed.body"),
        details,
      };
    case "backup.restore-warning":
      return {
        title: t("notifications.condition.backupRestoreWarning.title"),
        body: t("notifications.condition.backupRestoreWarning.body"),
        details,
      };
    case "pricing.rebuild-failed":
      return {
        title: t("notifications.condition.pricingRebuildFailed.title"),
        body: t("notifications.condition.pricingRebuildFailed.body"),
        details,
      };
    case "workspace.refresh-failed":
      return {
        title: t("notifications.condition.workspaceRefreshFailed.title"),
        body: t("notifications.condition.workspaceRefreshFailed.body"),
        details,
      };
    case "workspace.books-need-attention":
      return {
        title: t("notifications.condition.workspaceBooks.title"),
        body: first.params.blocking
          ? t("notifications.condition.workspaceBooks.blockingBody")
          : t("notifications.condition.workspaceBooks.body"),
        details,
      };
    case "watch.evidence-changed":
      return {
        title: t("notifications.condition.watchChanged.title"),
        body: t("notifications.condition.watchChanged.body"),
        details: [],
      };
    case "source-funds.rescan-needed":
      return {
        title: t("notifications.condition.sourceFundsRescan.title"),
        body: t("notifications.condition.sourceFundsRescan.body", {
          count: first.params.count ?? 1,
        }),
        details,
      };
  }
}

function journalsProcessedBody(entry: ResultEntry, t: ChromeT): string | undefined {
  const { params } = entry;
  const parts = [
    params.transactions !== undefined
      ? t("notifications.result.journalsProcessed.transactions", {
          count: params.transactions,
        })
      : null,
    params.entries !== undefined
      ? t("notifications.result.journalsProcessed.entries", {
          count: params.entries,
        })
      : null,
    params.quarantined
      ? t("notifications.result.journalsProcessed.quarantined", {
          count: params.quarantined,
        })
      : null,
    params.autoPaired
      ? t("notifications.result.journalsProcessed.autoPaired", {
          count: params.autoPaired,
        })
      : null,
    params.warnings
      ? t("notifications.result.journalsProcessed.warnings", {
          count: params.warnings,
        })
      : null,
  ].filter((part): part is string => Boolean(part));
  return parts.length ? parts.join(", ") : undefined;
}

export function projectResult(
  entry: ResultEntry,
  mode: PrivacyMode,
  t: ChromeT,
): ProjectedEntry {
  const details = projectDetails(entry.details, mode, t);
  switch (entry.code) {
    case "book-refresh.finished":
      return {
        title: entry.params.forceFull
          ? t("notifications.result.bookRefreshed.rescanTitle")
          : t("notifications.result.bookRefreshed.title"),
        details,
      };
    case "book-refresh.pending":
      return {
        title: t("notifications.result.bookRefreshPending.title"),
        body: t("notifications.result.bookRefreshPending.body"),
        details,
      };
    case "book-refresh.problems":
      return {
        title: t("notifications.result.bookRefreshProblems.title"),
        body: t("notifications.result.bookRefreshProblems.body"),
        details,
      };
    case "journals.processed":
      return {
        title: t("notifications.result.journalsProcessed.title"),
        body: journalsProcessedBody(entry, t),
        details,
      };
    case "connection.synced":
      return {
        title: entry.params.forceFull
          ? t("notifications.result.connectionSynced.rescanTitle")
          : t("notifications.result.connectionSynced.title"),
        body:
          entry.params.transactions !== undefined
            ? t("notifications.result.connectionSynced.imported", {
                count: entry.params.transactions,
              })
            : undefined,
        details,
      };
    case "backup.restored":
      return { title: t("notifications.result.backupRestored.title"), details };
    case "report.exported":
      return entry.severity === "warning"
        ? {
            title: t("notifications.result.reportExported.notesTitle"),
            body: t("notifications.result.reportExported.notesBody"),
            details,
          }
            : { title: t("notifications.result.reportExported.title"), details };
case "workspace.refreshed":
  return {
    title: t("notifications.result.workspaceRefreshed.title"),
    body:
      entry.params.count !== undefined && entry.params.entries !== undefined
        ? t("notifications.result.workspaceRefreshed.body", {
            synced: entry.params.count,
            count: entry.params.entries,
          })
        : undefined,
    details,
  };
case "pricing.rebuilt":
  return {
    title: t("notifications.result.pricingRebuilt.title"),
    body:
      entry.severity === "warning"
        ? t("notifications.result.pricingRebuilt.journalsBody")
        : undefined,
    details,
  };
  }
}

export function projectJob(
  job: JobEntry,
  mode: PrivacyMode,
  t: ChromeT,
): { title: string; label?: string } {
  const title = jobTitle(job, t);
  // A phase label is producer text that can name a source.
  const label =
    mode === "hideAll" ? undefined : job.progress.label?.trim() || undefined;
  return { title, label: label && label !== title ? label : undefined };
}

function jobTitle(job: JobEntry, t: ChromeT): string {
  switch (job.code) {
    case "journal-processing":
      return t("notifications.job.journalProcessing");
    case "book-refresh":
      return job.params.forceFull
        ? t("notifications.job.bookRescan")
        : t("notifications.job.bookRefresh");
    case "connection-sync":
      return job.params.forceFull
        ? t("notifications.job.connectionRescan")
        : t("notifications.job.connectionSync");
    case "report-export":
      return t("notifications.job.reportExport");
    case "pricing-rebuild":
      return t("notifications.job.pricingRebuild");
    case "workspace-refresh":
      return t("notifications.job.workspaceRefresh");
  }
}

function genericTitle(tone: NotificationTone | NotificationSeverity, t: ChromeT) {
  switch (tone) {
    case "error":
      return t("notifications.generic.failedTitle");
    case "warning":
      return t("notifications.generic.attentionTitle");
    case "success":
      return t("notifications.generic.doneTitle");
    default:
      return t("notifications.generic.noticeTitle");
  }
}

/**
 * A sender that has not moved to the structured model yet. Its title and
 * body are finished text of unknown content, so Hide all replaces them with
 * generic wording and Hide amounts withholds any that contain digits.
 */
export function projectLegacy(
  item: Pick<AppNotification, "title" | "body" | "tone" | "details">,
  mode: PrivacyMode,
  t: ChromeT,
): ProjectedEntry {
  if (mode === "hideAll") {
    return {
      title: genericTitle(item.tone, t),
      body: t("notifications.generic.body"),
      details: [],
    };
  }
  const title =
    mode === "hideAmounts" && mayHoldAmount(item.title)
      ? genericTitle(item.tone, t)
      : item.title;
  return {
    title,
    body: projectFreeText(item.body, mode, t),
    details: (item.details ?? [])
      .map((line) => projectFreeText(line, mode, t))
      .filter((line): line is string => Boolean(line)),
  };
}
