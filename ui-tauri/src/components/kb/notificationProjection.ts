import type { TFunction } from "i18next";

import type { PrivacyMode } from "@/store/ui";
import type {
  ConditionCode,
  ConditionGroup,
  JobEntry,
  NotificationDetail,
  ResultEntry,
} from "@/store/notifications";
import { redactCredentials } from "@/lib/secretScanner";

import { groupDetails, type DetailGroup } from "./notificationCauses";
import {
  projectFreeText,
  projectLabel,
} from "./notificationPrivacy";

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
  /** Detail lines, one per cause: what went wrong and where. */
  details: string[];
  /**
   * The raw messages behind classified causes, for a bug report. Shown only
   * on request, under the same privacy rules as any free text.
   */
  technical: string[];
}

type ChromeT = TFunction<"chrome">;

/**
 * Names in a line, only when there are few: more than this becomes a
 * count, and the entry's action opens the full list.
 */
const NAMES_SHOWN = 2;

function nameList(labels: readonly string[], mode: PrivacyMode): string | undefined {
  if (labels.length === 0 || labels.length > NAMES_SHOWN) return undefined;
  const names = labels
    .map((label) => projectLabel(label, mode))
    .filter((name): name is string => Boolean(name));
  return names.length ? names.join(", ") : undefined;
}

function groupLine(group: DetailGroup, mode: PrivacyMode, t: ChromeT): string | null {
  const names = nameList(group.labels, mode);
  let text: string | undefined;
  if (group.cause) {
    text = t(`notifications.cause.${group.cause}`);
  } else {
    const phrase = group.phraseKey
      ? t(`notifications.phrase.${group.phraseKey}`)
      : undefined;
    const message = projectFreeText(group.message, mode, t);
    text = [phrase, message].filter(Boolean).join(" — ") || undefined;
  }
  if (text && names) {
    // "Cause: wallets" for a known cause; "wallets: message" otherwise.
    return group.cause
      ? t("notifications.detailLine", { label: text, message: names })
      : t("notifications.detailLine", { label: names, message: text });
  }
  if (text && group.count > 1) {
    return t("notifications.causeCount", { cause: text, count: group.count });
  }
  return text ?? names ?? null;
}

/** Every raw text behind the entry, for Copy error details. */
function technicalLines(
  groups: readonly DetailGroup[],
  mode: PrivacyMode,
  t: ChromeT,
): string[] {
  const lines = new Map<string, number>();
  for (const group of groups) {
    for (const item of group.messages) {
      const text = projectFreeText(item.text, mode, t);
      if (!text) continue;
      lines.set(text, (lines.get(text) ?? 0) + item.count);
    }
  }
  return [...lines].map(([text, count]) =>
    count > 1 ? t("notifications.technicalRepeated", { message: text, count }) : text,
  );
}

function projectDetails(
  details: readonly NotificationDetail[] | undefined,
  mode: PrivacyMode,
  t: ChromeT,
): { details: string[]; technical: string[] } {
  if (mode === "hideAll" || !details?.length) return { details: [], technical: [] };
  const groups = groupDetails(details);
  return {
    details: groups
      .map((group) => groupLine(group, mode, t))
      .filter((line): line is string => Boolean(line)),
    technical: technicalLines(groups, mode, t),
  };
}

/** One attention row: what is wrong and what it means for reports. */
export function projectConditionGroup(
  group: ConditionGroup,
  mode: PrivacyMode,
  t: ChromeT,
): ProjectedEntry {
  const first = group.entries[0];
  const { details, technical } = projectDetails(
    group.entries.flatMap((entry) => entry.details ?? []),
    mode,
    t,
  );
  return { technical, ...projectConditionText(group, first, details, mode, t) };
}

function projectConditionText(
  group: ConditionGroup,
  first: ConditionGroup["entries"][number],
  details: string[],
  mode: PrivacyMode,
  t: ChromeT,
): Omit<ProjectedEntry, "technical"> {
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
    case "connection.sync-failed": {
      // One wallet names itself in the title, where the privacy mode allows.
      const name =
        group.entries.length === 1
          ? projectLabel(first.params.sources?.[0], mode)
          : undefined;
      return {
        title: name
          ? t("notifications.condition.connectionSyncFailed.titleNamed", { name })
          : t("notifications.condition.connectionSyncFailed.title", {
              count: group.entries.length,
            }),
        body: t("notifications.condition.connectionSyncFailed.body"),
        details,
      };
    }
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
  if (entry.code === "activity.new") return projectNewActivity(entry, mode, t);
  const { details, technical } = projectDetails(entry.details, mode, t);
  return { technical, ...projectResultText(entry, details, mode, t) };
}

/**
 * New transactions: how many, which way, and in which wallets. Counts are
 * typed and show in every mode; wallet names follow `projectLabel`.
 */
/** Wallet lines a "new transactions" entry lists before "+N more". */
const WALLETS_SHOWN = 4;

function projectNewActivity(
  entry: ResultEntry,
  mode: PrivacyMode,
  t: ChromeT,
): ProjectedEntry {
  const { params } = entry;
  const count = params.transactions ?? 0;
  const name =
    params.sources?.length === 1 ? projectLabel(params.sources[0], mode) : undefined;
  const directions = [
    params.incoming
      ? t("notifications.result.newActivity.incoming", { count: params.incoming })
      : null,
    params.outgoing
      ? t("notifications.result.newActivity.outgoing", { count: params.outgoing })
      : null,
  ].filter((part): part is string => Boolean(part));
  const wallets = params.walletCounts ?? [];
  const details: string[] = [];
  if (wallets.length > 1 && mode !== "hideAll") {
    for (const wallet of wallets.slice(0, WALLETS_SHOWN)) {
      const label = projectLabel(wallet.label, mode);
      if (!label) continue;
      details.push(
        t("notifications.detailLine", {
          label,
          message: t("notifications.result.newActivity.walletCount", {
            count: wallet.count,
          }),
        }),
      );
    }
    if (wallets.length > WALLETS_SHOWN) {
      details.push(
        t("notifications.result.newActivity.moreWallets", {
          count: wallets.length - WALLETS_SHOWN,
        }),
      );
    }
  }
  return {
    title: name
      ? t("notifications.result.newActivity.titleNamed", { count, name })
      : t("notifications.result.newActivity.title", { count }),
    body: directions.length ? directions.join(", ") : undefined,
    details,
    technical: [],
  };
}

function projectResultText(
  entry: ResultEntry,
  details: string[],
  mode: PrivacyMode,
  t: ChromeT,
): Omit<ProjectedEntry, "technical"> {
  switch (entry.code) {
    case "activity.new":
      return projectNewActivity(entry, mode, t);
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
  // A phase label is producer text that can name a source, and a source
  // name can carry an amount.
  const label = projectFreeText(job.progress.label, mode, t);
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

/**
 * Problems the assistant can help troubleshoot: failures with a cause it
 * can look into. Not states with their own one-click fix or dedicated flow
 * (review, stale journals, Touch ID) and not results.
 */
export const ASSISTANT_TROUBLESHOOT_CODES: readonly ConditionCode[] = [
  "source.refresh-failed",
  "freshness.worker-error",
  "book-refresh.failed",
  "journals.process-failed",
  "connection.sync-failed",
  "btcpay.provenance-incomplete",
  "backup.restore-failed",
  "pricing.rebuild-failed",
  "workspace.refresh-failed",
];

/**
 * A wallet name the assistant may see: a plain name. A label that looks
 * like a location (a path, URL, domain, IP address, or host:port) or that
 * credential redaction would change is configuration, not a name, and is
 * left out.
 */
const LOCATION_LIKE =
  /[/\\~]|:\/\/|\b\d{1,3}(\.\d{1,3}){3}\b|\b[a-z0-9-]+(\.[a-z0-9-]+)+\b|:\d{2,5}\b|@/i;
export function assistantSafeName(label: string): string | null {
  const trimmed = label.trim();
  if (!trimmed || LOCATION_LIKE.test(trimmed)) return null;
  return redactCredentials(trimmed) === trimmed ? trimmed : null;
}

/**
 * The question handed to the assistant. Built from fixed, translated text
 * only: the entry's generic title and body (never a title that names a
 * wallet), each cause in plain words, and plain wallet names. Raw daemon
 * messages and hints never go: they can carry paths, hosts, ports, or URLs,
 * which are configuration the assistant does not get. An unclassified
 * error, and a name that looks like a location, are only counted.
 */
export function assistantTroubleshootPrompt(
  group: ConditionGroup,
  mode: PrivacyMode,
  t: ChromeT,
): string {
  // Hide all's projection is the generic wording: no label in title or body.
  const generic = projectConditionGroup(group, "hideAll", t);
  const lines = groupDetails(group.entries.flatMap((entry) => entry.details ?? [])).map(
    (detailGroup) => {
      if (!detailGroup.cause) {
        return t("notifications.assistantOtherError", { count: detailGroup.count });
      }
      const cause = t(`notifications.cause.${detailGroup.cause}`);
      const names =
        mode === "hideAll"
          ? []
          : detailGroup.labels
              .map((label) => projectLabel(label, mode))
              .filter((name): name is string => Boolean(name))
              .map((name) => assistantSafeName(name))
              .filter((name): name is string => Boolean(name));
      const withheld = detailGroup.count - names.length;
      const listed = names.length
        ? t("notifications.detailLine", { label: cause, message: names.join(", ") })
        : cause;
      return withheld > 0 && names.length
        ? t("notifications.assistantMoreSources", { line: listed, count: withheld })
        : names.length
          ? listed
          : detailGroup.count > 1
            ? t("notifications.causeCount", { cause, count: detailGroup.count })
            : cause;
    },
  );
  const seen = [generic.body, ...lines.map((line) => `- ${line}`)]
    .filter(Boolean)
    .join("\n");
  return t("notifications.assistantPrompt", { title: generic.title, seen });
}
