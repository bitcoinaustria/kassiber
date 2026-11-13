import { create } from "zustand";

import type { ActiveBook } from "@/lib/activeBook";
import type { NotificationTarget } from "@/components/kb/notificationRouting";
import { resolveEntryAction } from "@/components/kb/notificationRouting";
import type { FreshnessRunData } from "@/lib/syncResults";
import type {
  ConditionCode,
  ConditionEntry,
  NewOccurrence,
  NotificationActionSpec,
  NotificationScope,
  ResultEntry,
} from "@/store/notifications";

/**
 * Opt-in OS (desktop) notifications. Every decision about *whether* an event
 * may reach the operating system lives in `osNotificationEligibility`; what
 * the notification says is fixed in Rust (`src-tauri/src/os_notifications.rs`)
 * and cannot be influenced from here. Only an allowlisted category and an
 * opaque random token cross the boundary. The token's destination stays in
 * this module's RAM and is checked against the open book after unlock.
 */

/** Mirrors `Category` in `os_notifications.rs`. */
export const OS_NOTIFICATION_CATEGORIES = [
  "needs-attention",
  "job-finished",
  "new-activity",
  "evidence-changed",
] as const;
export type OsNotificationCategory = (typeof OS_NOTIFICATION_CATEGORIES)[number];

export const OS_NOTIFICATION_ACTIVATED_EVENT = "os-notification://activated";

/** The same event (same key) is not delivered twice within this window. */
export const DEDUPE_WINDOW_MS = 30 * 60_000;
/** After any delivery, nothing else is delivered for this long. */
export const BURST_COOLDOWN_MS = 60_000;
/** A clicked token older than this opens the generic center instead. */
export const TOKEN_TTL_MS = 10 * 60_000;
const MAX_PENDING_TOKENS = 16;

/** Where an activated notification leads. RAM only. */
export type OsNotificationDestination =
  | { kind: "center" }
  | {
      kind: "entry";
      scope: NotificationScope;
      action?: NotificationActionSpec;
      ref?: { lane: "condition"; key: string } | { lane: "result"; id: string };
    };

export interface OsNotificationCandidate {
  category: OsNotificationCategory;
  /** Stable identity for dedupe (a condition key, a result code + book). */
  dedupeKey: string;
  /** False for anything rebuilt, reconciled, or repeated before being seen. */
  isNew: boolean;
  destination: OsNotificationDestination;
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
]);

function entryDestination(
  entry: ConditionEntry | ResultEntry,
  ref: { lane: "condition"; key: string } | { lane: "result"; id: string },
): OsNotificationDestination {
  return {
    kind: "entry",
    scope: { ...entry.scope },
    ...(entry.action ? { action: entry.action } : {}),
    ref,
  };
}

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
      destination: entryDestination(occurrence.entry, {
        lane: "condition",
        key: occurrence.key,
      }),
    };
  }
  const { entry } = occurrence;
  return {
    category: entry.severity === "info" ? "job-finished" : "needs-attention",
    dedupeKey: `result:${JSON.stringify([entry.code, entry.scope.book])}`,
    isNew: true,
    destination: entryDestination(entry, { lane: "result", id: entry.id }),
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
    destination: {
      kind: "entry",
      scope: { book },
      action: { type: "navigate", to: "/chain-analysis" },
    },
  };
}

/**
 * Whether a background run committed new observations: a finished job that
 * reports imported rows. Fetched, updated, or unchanged rows are not new.
 */
export function backgroundRunHasNewObservations(
  data: FreshnessRunData | null | undefined,
): boolean {
  return (data?.completed ?? []).some((job) => {
    if (job.status !== "done") return false;
    const imported = job.result?.imported;
    return typeof imported === "number" && imported > 0;
  });
}

export function newActivityCandidate(
  book: string | null,
): OsNotificationCandidate {
  return {
    category: "new-activity",
    dedupeKey: `new-activity:${JSON.stringify(book)}`,
    isNew: true,
    destination: {
      kind: "entry",
      scope: { book },
      action: { type: "navigate", to: "/transactions" },
    },
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
  token: () => string;
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

/** 128 random bits, hex: opaque, and shaped so it cannot carry prose. */
export function randomToken(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
}

let deps: OsNotificationDeps | null = null;
let lastDeliveredAt: number | null = null;
const lastDeliveredByKey = new Map<string, number>();
const pendingTokens = new Map<
  string,
  { destination: OsNotificationDestination; createdAt: number }
>();
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
    token: randomToken,
  };
}

/** Forget every delivery and token: a fresh session, and tests. */
export function resetOsNotifications(): void {
  deps = null;
  lastDeliveredAt = null;
  lastDeliveredByKey.clear();
  pendingTokens.clear();
  queue = Promise.resolve();
  useOsNotificationActivation.setState({ pending: null });
}

function pruneTokens(now: number): void {
  for (const [token, entry] of pendingTokens) {
    if (now - entry.createdAt > TOKEN_TTL_MS) pendingTokens.delete(token);
  }
  while (pendingTokens.size > MAX_PENDING_TOKENS) {
    const oldest = pendingTokens.keys().next().value;
    if (oldest === undefined) break;
    pendingTokens.delete(oldest);
  }
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

  const token = live.token();
  let outcome: unknown;
  try {
    // Only the category and the token: Rust writes the text.
    outcome = await live.invoke("deliver_os_notification", {
      category: candidate.category,
      token,
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
  pendingTokens.set(token, { destination: candidate.destination, createdAt: now });
  pruneTokens(now);
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
  const next = queue.then(() => offerNow(candidate));
  queue = next.catch(() => undefined);
  return next;
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

/* ------------------------------------------------------------------------ */
/* Activation                                                                */
/* ------------------------------------------------------------------------ */

/**
 * The destination for an activated token, consumed once. An unknown token
 * (after a restart) or an expired one opens the generic center.
 */
export function destinationForActivation(
  token: unknown,
  now: number = Date.now(),
): OsNotificationDestination {
  if (typeof token !== "string") return { kind: "center" };
  const entry = pendingTokens.get(token);
  pendingTokens.delete(token);
  if (!entry || now - entry.createdAt > TOKEN_TTL_MS) return { kind: "center" };
  return entry.destination;
}

/** A pending activation waits here (RAM only) until the shell is unlocked. */
export const useOsNotificationActivation = create<{
  pending: OsNotificationDestination | null;
}>()(() => ({ pending: null }));

export function recordOsNotificationActivation(
  token: unknown,
  now: number = Date.now(),
): void {
  useOsNotificationActivation.setState({
    pending: destinationForActivation(token, now),
  });
}

export type OsNotificationRoute =
  | { kind: "center" }
  | { kind: "navigate"; to: NotificationTarget };

/**
 * Where an activation goes, checked against the book open now. Only
 * navigation: an operation's destination is its review page, never a rerun.
 * Anything stale, gone, or unknown opens the center.
 */
export function resolveActivation(
  destination: OsNotificationDestination,
  context: {
    currentBook: ActiveBook | null;
    developerToolsEnabled: boolean;
    conditions: Record<string, ConditionEntry>;
    results: readonly ResultEntry[];
  },
): OsNotificationRoute {
  if (destination.kind !== "entry") return { kind: "center" };
  const { scope, action, ref } = destination;
  if (!context.currentBook || scope.book !== context.currentBook.key) {
    return { kind: "center" };
  }
  if (ref?.lane === "condition" && !(ref.key in context.conditions)) {
    return { kind: "center" };
  }
  if (
    ref?.lane === "result" &&
    !context.results.some((entry) => entry.id === ref.id)
  ) {
    return { kind: "center" };
  }
  const navigation: NotificationActionSpec | undefined =
    action?.type === "operation" ? { type: "navigate", to: action.review } : action;
  const resolved = resolveEntryAction(
    navigation,
    scope,
    context.currentBook,
    context.developerToolsEnabled,
  );
  if (!resolved || resolved.kind !== "navigate" || resolved.stale) {
    return { kind: "center" };
  }
  return { kind: "navigate", to: resolved.to };
}
