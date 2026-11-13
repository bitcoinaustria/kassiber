import type { ActiveBook } from "@/lib/activeBook";
import { newActivityFromRun, recordNewActivity } from "@/lib/newActivity";
import type { DaemonFreshnessEvent } from "@/lib/daemonFreshnessEvent";
import type { ReviewBadges } from "@/lib/reviewCount";
import { reviewTotal } from "@/lib/reviewCount";
import { redactCredentials } from "@/lib/secretScanner";
import type {
  FreshnessJobSummary,
  FreshnessRunData,
  FreshnessSourceState,
} from "@/lib/syncResults";
import {
  conditionKey,
  useNotificationStore,
  type ConditionInput,
  type NotificationDetail,
  type NotificationScope,
} from "@/store/notifications";

/**
 * Turning authoritative book state into center conditions. Pure functions:
 * the callers decide when a snapshot is authoritative (a successful read or
 * event), and these decide what it proves.
 *
 * A source resolves only when a snapshot reports it `fresh` and not blocking
 * reports. Queued, syncing, paused, rate-limited, or absent sources prove
 * nothing, so their conditions stay.
 */

/** Operational text for a detail line: scrubbed again, bounded by the store. */
export function safeMessage(text: string | null | undefined): string | undefined {
  const trimmed = text?.trim();
  if (!trimmed) return undefined;
  return redactCredentials(trimmed);
}

function sourceIdentity(source: {
  source_key?: string;
  source_label?: string;
}): string | null {
  if (source.source_key) return source.source_key;
  if (source.source_label) return `label:${source.source_label}`;
  return null;
}

const REVIEW_TARGET = "/connections";

function sourceCondition(
  book: string | null,
  source: string,
  options: {
    label?: string;
    message?: string;
    failed: boolean;
    blocking: boolean;
    phrase?: NotificationDetail["phraseKey"];
  },
): ConditionInput {
  return {
    code: "source.refresh-failed",
    scope: { book, source },
    severity: options.failed ? "error" : "warning",
    priority: options.blocking ? "blocking" : "attention",
    params: {
      blocking: options.blocking,
      ...(options.label ? { sources: [options.label] } : {}),
    },
    action: { type: "navigate", to: REVIEW_TARGET },
    details: [
      {
        label: options.label,
        message: safeMessage(options.message),
        phraseKey:
          options.phrase ?? (options.failed ? "failed" : "blocksReports"),
      },
    ],
  };
}

function sourceNeedsAttention(source: FreshnessSourceState): boolean {
  return source.status === "failed" || Boolean(source.blocking_reports);
}

/**
 * What a freshness source snapshot proves for one book: failing sources are
 * present, fresh ones are resolved. `sources` must be the complete snapshot
 * list (a status read or a run/event that carries it).
 */
export function sourceConditionsFromSnapshot(
  sources: readonly FreshnessSourceState[],
  book: string | null,
): { present: ConditionInput[]; resolved: string[] } {
  const present: ConditionInput[] = [];
  const resolved: string[] = [];
  for (const source of sources) {
    const identity = sourceIdentity(source);
    if (!identity) continue;
    if (sourceNeedsAttention(source)) {
      present.push(
        sourceCondition(book, identity, {
          label: source.source_label || undefined,
          message: source.last_error_message ?? undefined,
          failed: source.status === "failed",
          blocking: Boolean(source.blocking_reports),
        }),
      );
    } else if (source.status === "fresh") {
      resolved.push(
        conditionKey("source.refresh-failed", { book, source: identity }),
      );
    }
  }
  return { present, resolved };
}

function jobFailed(job: FreshnessJobSummary): boolean {
  return job.status === "error" || job.status === "cancelled";
}

/**
 * Source failures a run or background event reports as attempts: completed
 * jobs that errored or were cancelled. Each is a new occurrence for its
 * source; a failing source the snapshot merely lists is reconciled instead.
 */
export function sourceFailuresFromRun(
  data: FreshnessRunData | null | undefined,
  book: string | null,
): ConditionInput[] {
  const failures = new Map<string, ConditionInput>();
  const blockingSources = new Set(
    (data?.sources ?? [])
      .filter((source) => source.blocking_reports)
      .map((source) => sourceIdentity(source)),
  );
  for (const job of data?.completed ?? []) {
    if (!jobFailed(job) || job.job_type === "journal_refresh") continue;
    const identity = sourceIdentity(job);
    if (!identity) continue;
    failures.set(
      identity,
      sourceCondition(book, identity, {
        label: job.source_label || undefined,
        message: [job.error?.message || job.error?.code, job.error?.hint]
          .filter(Boolean)
          .join(" "),
        failed: true,
        blocking: blockingSources.has(identity),
        phrase: job.status === "cancelled" ? "cancelled" : "failed",
      }),
    );
  }
  return [...failures.values()];
}

/** Whether a run or event carried a complete source snapshot. */
export function hasSourceSnapshot(
  data: FreshnessRunData | null | undefined,
): data is FreshnessRunData & { sources: FreshnessSourceState[] } {
  return Array.isArray(data?.sources);
}

/**
 * The live journal and review conditions from `ui.review.badges`. Call only
 * with a successful read: a missing or failed read resolves nothing.
 */
export function reviewConditionsFromBadges(
  badges: ReviewBadges,
  book: string | null,
): { present: ConditionInput[]; resolved: string[] } {
  const scope: NotificationScope = { book };
  const present: ConditionInput[] = [];
  const resolved: string[] = [];
  if (badges.journals_needs_processing) {
    // One click fixes it, so it leads the attention band rather than
    // counting as a blocker (the bell stays amber for it, as before).
    present.push({
      code: "journals.stale",
      scope,
      severity: "warning",
      priority: "attention",
      action: {
        type: "operation",
        operation: { kind: "process-journals", autoPair: true },
        review: "/journals",
      },
    });
  } else {
    resolved.push(conditionKey("journals.stale", scope));
  }
  const count = reviewTotal(badges);
  const blocking = (badges.quarantine ?? 0) > 0;
  if (count > 0) {
    present.push({
      code: "review.pending",
      scope,
      severity: "warning",
      priority: blocking ? "blocking" : "attention",
      params: { count, blocking },
      action: { type: "navigate", to: "/review" },
    });
  } else {
    resolved.push(conditionKey("review.pending", scope));
  }
  return { present, resolved };
}

/**
 * Explicit evidence that a run processed the book's journals: its journal
 * step reports `processed` or `current`, or a `journal_refresh` job finished
 * `done`. Anything else (a deferred, disabled, failed, or absent step)
 * proves nothing about an earlier journal failure.
 */
export function journalStepSucceeded(data: unknown): boolean {
  if (!data || typeof data !== "object") return false;
  const run = data as {
    journals?: { status?: unknown } | null;
    completed?: unknown;
  };
  const status = run.journals?.status;
  if (status === "processed" || status === "current") return true;
  return (
    Array.isArray(run.completed) &&
    run.completed.some(
      (job) =>
        job &&
        typeof job === "object" &&
        (job as { job_type?: unknown }).job_type === "journal_refresh" &&
        (job as { status?: unknown }).status === "done",
    )
  );
}

/**
 * Apply what a finished freshness run (foreground or background) proves for
 * `book`: failed attempts count as new occurrences, the snapshot it carries
 * reconciles the rest, and sources it reports fresh resolve. A run without a
 * snapshot resolves nothing.
 */
export function applyFreshnessRun(
  data: FreshnessRunData | null | undefined,
  book: string | null,
): void {
  const store = useNotificationStore.getState();
  if (journalStepSucceeded(data)) {
    store.resolveCondition(conditionKey("journals.process-failed", { book }));
  }
  const failures = sourceFailuresFromRun(data, book);
  for (const failure of failures) store.raiseCondition(failure);
  if (!hasSourceSnapshot(data)) return;
  const raised = new Set(failures.map((input) => input.scope.source));
  const snapshot = sourceConditionsFromSnapshot(data.sources, book);
  store.reconcileConditions({
    present: snapshot.present.filter((input) => !raised.has(input.scope.source)),
    resolved: snapshot.resolved.filter(
      (key) => !failures.some((input) => conditionKey(input.code, input.scope) === key),
    ),
  });
}

/**
 * Apply an unsolicited freshness event, but only to the book it ran for.
 * The listener receives events for whichever book the daemon had open when
 * the work ran; one delayed past a switch, or one that names no book, must
 * not raise or clear anything in the book open now. Those events are left to
 * the caller's re-read of the open book's state. Returns whether the event
 * was applied.
 */
export function applyFreshnessEvent(
  event: DaemonFreshnessEvent,
  active: ActiveBook | null,
): boolean {
  if (!active || !event.profileId || event.profileId !== active.expected.profile_id) {
    return false;
  }
  const scope = { book: active.key, expected: active.expected };
  const store = useNotificationStore.getState();
  if (event.kind === "worker-error") {
    store.raiseCondition({
      code: "freshness.worker-error",
      scope,
      severity: "error",
      priority: "attention",
      action: { type: "navigate", to: "/connections" },
      details: [{ message: safeMessage(event.message ?? event.code) }],
    });
    return true;
  }
  // A background run that reported back means the worker runs again; each
  // source is then settled by what this run proves for it.
  store.resolveCondition(conditionKey("freshness.worker-error", scope));
  applyFreshnessRun(event.data, active.key);
  recordNewActivity(newActivityFromRun(event.data), scope);
  return true;
}
