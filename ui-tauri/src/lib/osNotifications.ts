import type {
  ConditionCode,
  NewOccurrence,
  ResultCode,
} from "@/store/notifications";

/**
 * Opt-in OS (desktop) notifications. Every decision about *whether* an event
 * may reach the operating system lives in `osNotificationEligibility`; what
 * the notification says is fixed in Rust (`src-tauri/src/os_notifications.rs`)
 * and cannot be influenced from here. Only an allowlisted category crosses
 * the boundary. A click brings Kassiber to the front, nothing more: the
 * desktop plugin does not report clicks, and nothing is guessed from focus.
 */

/** Mirrors `Category` in `os_notifications.rs`. */
export const OS_NOTIFICATION_CATEGORIES = [
  "needs-attention",
  "job-finished",
  "new-activity",
  "evidence-changed",
] as const;
export type OsNotificationCategory = (typeof OS_NOTIFICATION_CATEGORIES)[number];

/** The same event (same key) is not delivered twice within this window. */
export const DEDUPE_WINDOW_MS = 30 * 60_000;
/** After any delivery, nothing else is delivered for this long. */
export const BURST_COOLDOWN_MS = 60_000;
export interface OsNotificationCandidate {
  category: OsNotificationCategory;
  /** Stable identity for dedupe (a condition key, a result code + book). */
  dedupeKey: string;
  /** False for anything rebuilt, reconciled, or repeated before being seen. */
  isNew: boolean;
}

export type OsNotificationSkipReason =
  | "disabled"
  | "unavailable"
  | "category"
  | "not-new"
  | "focused"
  | "duplicate"
  | "cooldown";

export type OsNotificationDecision =
  | { eligible: true }
  | { eligible: false; reason: OsNotificationSkipReason };

export interface OsNotificationContext {
  /** Settings → Privacy → Desktop notifications. */
  enabled: boolean;
  /** Running inside the native shell. */
  available: boolean;
  /** The native window's focus; null (unknown) counts as focused. */
  windowFocused: boolean | null;
  now: number;
  lastDeliveredAt: number | null;
  lastDeliveredByKey: ReadonlyMap<string, number>;
}

/**
 * The one place that decides whether an event may become an OS
 * notification. Visibility is not focus: a visible window behind another
 * app qualifies, a focused one never does.
 */
export function osNotificationEligibility(
  candidate: OsNotificationCandidate,
  context: OsNotificationContext,
): OsNotificationDecision {
  if (!context.enabled) return { eligible: false, reason: "disabled" };
  if (!context.available) return { eligible: false, reason: "unavailable" };
  if (
    !(OS_NOTIFICATION_CATEGORIES as readonly string[]).includes(
      candidate.category,
    )
  ) {
    return { eligible: false, reason: "category" };
  }
  if (!candidate.isNew) return { eligible: false, reason: "not-new" };
  if (context.windowFocused !== false) {
    return { eligible: false, reason: "focused" };
  }
  const lastForKey = context.lastDeliveredByKey.get(candidate.dedupeKey);
  if (lastForKey !== undefined && context.now - lastForKey < DEDUPE_WINDOW_MS) {
    return { eligible: false, reason: "duplicate" };
  }
  if (
    context.lastDeliveredAt !== null &&
    context.now - context.lastDeliveredAt < BURST_COOLDOWN_MS
  ) {
    return { eligible: false, reason: "cooldown" };
  }
  return { eligible: true };
}

/**
 * Raised conditions that are background outcomes needing action. The live
 * journal and review conditions are only ever reconciled from a snapshot
 * (unchanged backlog), so they are not here.
 */
const ATTENTION_CONDITION_CODES: ReadonlySet<ConditionCode> = new Set([
  "source.refresh-failed",
  "freshness.worker-error",
  "book-refresh.failed",
  "book-refresh.backend-required",
  "journals.process-failed",
  "connection.sync-failed",
  "pricing.rebuild-failed",
  "workspace.refresh-failed",
  "backup.restore-failed",
]);

/**
 * Results that mean background work is done. `book-refresh.pending` is not:
 * its sources or journals are still queued.
 */
const FINISHED_RESULT_CODES: ReadonlySet<ResultCode> = new Set([
  "book-refresh.finished",
  "journals.processed",
  "connection.synced",
  "report.exported",
  "pricing.rebuilt",
  "workspace.refreshed",
  "backup.restored",
]);

/**
 * What a new center occurrence offers the OS. A condition is news when it is
 * new or was already seen; a repeat the user has not looked at yet is not.
 */
export function candidateForOccurrence(
  occurrence: NewOccurrence,
): OsNotificationCandidate | null {
  if (occurrence.lane === "condition") {
    if (!ATTENTION_CONDITION_CODES.has(occurrence.entry.code)) return null;
    return {
      category: "needs-attention",
      dedupeKey: `condition:${occurrence.key}`,
      isNew: !occurrence.previous || Boolean(occurrence.previous.seenAt),
    };
  }
  const { entry, previous } = occurrence;
  // New transactions are their own category ("New activity in Kassiber"),
  // once per entry: more transactions merged into an entry the center has
  // not shown yet are not news again.
  if (entry.code === "activity.new") {
    return {
      category: "new-activity",
      dedupeKey: `new-activity:${JSON.stringify(entry.scope.book)}`,
      isNew: !previous || Boolean(previous.seenAt),
    };
  }
  if (entry.severity === "info" && !FINISHED_RESULT_CODES.has(entry.code)) {
    // Not finished (work still queued) or not background work at all.
    return null;
  }
  return {
    category: entry.severity === "info" ? "job-finished" : "needs-attention",
    dedupeKey: `result:${JSON.stringify([entry.code, entry.scope.book])}`,
    isNew: true,
  };
}

/** A local evidence watch recorded a change (pushed by the daemon). */
export function evidenceWatchCandidate(
  book: string | null,
): OsNotificationCandidate {
  return {
    category: "evidence-changed",
    dedupeKey: `evidence:${JSON.stringify(book)}`,
    isNew: true,
  };
}

/* ------------------------------------------------------------------------ */
/* Delivery                                                                  */
/* ------------------------------------------------------------------------ */

export interface OsNotificationDeps {
  enabled: () => boolean;
  available: () => boolean;
  windowFocused: () => Promise<boolean | null>;
  invoke: (command: string, args: Record<string, unknown>) => Promise<unknown>;
  now: () => number;
}

function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function nativeWindowFocused(): Promise<boolean | null> {
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return await getCurrentWindow().isFocused();
  } catch {
    return null;
  }
}

async function tauriInvoke(
  command: string,
  args: Record<string, unknown>,
): Promise<unknown> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke(command, args);
}

let deps: OsNotificationDeps | null = null;
let lastDeliveredAt: number | null = null;
const lastDeliveredByKey = new Map<string, number>();
let queue: Promise<unknown> = Promise.resolve();

/** Wire the live dependencies once (AppShell); tests pass their own. */
export function configureOsNotifications(next: OsNotificationDeps): void {
  deps = next;
}

export function liveOsNotificationDeps(enabled: () => boolean): OsNotificationDeps {
  return {
    enabled,
    available: isTauri,
    windowFocused: nativeWindowFocused,
    invoke: tauriInvoke,
    now: () => Date.now(),
  };
}

/** Forget every delivery: a fresh session, and tests. */
export function resetOsNotifications(): void {
  deps = null;
  lastDeliveredAt = null;
  lastDeliveredByKey.clear();
  queue = Promise.resolve();
  batch = [];
}

async function offerNow(
  candidate: OsNotificationCandidate,
): Promise<OsNotificationDecision> {
  const live = deps;
  if (!live) return { eligible: false, reason: "unavailable" };
  const base = {
    enabled: live.enabled(),
    available: live.available(),
    now: live.now(),
    lastDeliveredAt,
    lastDeliveredByKey,
  };
  // Ask the window only when nothing else already rules the event out.
  const precheck = osNotificationEligibility(candidate, {
    ...base,
    windowFocused: false,
  });
  if (!precheck.eligible) return precheck;
  const decision = osNotificationEligibility(candidate, {
    ...base,
    windowFocused: await live.windowFocused(),
  });
  if (!decision.eligible) return decision;

  let outcome: unknown;
  try {
    // Only the category: Rust writes the text.
    outcome = await live.invoke("deliver_os_notification", {
      category: candidate.category,
    });
  } catch {
    return { eligible: false, reason: "unavailable" };
  }
  if (outcome !== "delivered") {
    return {
      eligible: false,
      reason:
        outcome === "focused" || outcome === "cooldown" || outcome === "disabled"
          ? outcome
          : "unavailable",
    };
  }
  const now = live.now();
  lastDeliveredAt = now;
  lastDeliveredByKey.set(candidate.dedupeKey, now);
  return decision;
}

/**
 * Offer an event to the OS. Offers are decided one at a time, so a burst
 * cannot slip several notifications past the cooldown.
 */
export function offerOsNotification(
  candidate: OsNotificationCandidate | null,
): Promise<OsNotificationDecision> {
  if (!candidate) {
    return Promise.resolve({ eligible: false, reason: "category" });
  }
  return new Promise((resolve, reject) => {
    if (batch.length === 0) queueMicrotask(flushBatch);
    batch.push({ candidate, resolve, reject });
  });
}

/**
 * Which of several events from one moment wins the cooldown: what needs
 * the user first, then a watch change, new activity, and a finished job.
 */
const CATEGORY_PRIORITY: Record<OsNotificationCategory, number> = {
  "needs-attention": 0,
  "evidence-changed": 1,
  "new-activity": 2,
  "job-finished": 3,
};

interface PendingOffer {
  candidate: OsNotificationCandidate;
  resolve: (decision: OsNotificationDecision) => void;
  reject: (error: unknown) => void;
}

/**
 * Offers made in the same task (a refresh that files new transactions and a
 * failure together) are decided as one batch, most important first, so the
 * cooldown never lets a "finished" or "new activity" notice hide a failure.
 */
let batch: PendingOffer[] = [];

function flushBatch(): void {
  const current = batch;
  batch = [];
  current.sort(
    (a, b) =>
      CATEGORY_PRIORITY[a.candidate.category] - CATEGORY_PRIORITY[b.candidate.category],
  );
  for (const item of current) {
    const next = queue.then(() => offerNow(item.candidate));
    queue = next.catch(() => undefined);
    next.then(item.resolve, item.reject);
  }
}

/** Mirror the opt-in and UI language to the native side. */
export function syncOsNotificationPreferences(
  enabled: boolean,
  lang: string,
): Promise<void> {
  const live = deps;
  if (!live?.available()) return Promise.resolve();
  return live
    .invoke("set_os_notification_preferences", {
      enabled,
      language: lang === "de" ? "de" : "en",
    })
    .then(() => undefined)
    .catch(() => undefined);
}
