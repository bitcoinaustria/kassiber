import { create } from "zustand";

/**
 * The notification center's model, kept in RAM only (never in `kb.ui`).
 *
 * Three lanes that never compete for slots:
 *
 * - **Conditions**: what still needs attention, derived from authoritative
 *   state and keyed by scope + code + source. They stay until a successful
 *   snapshot for the same scope and source says otherwise; nothing evicts
 *   them, and Clear recent leaves them alone.
 * - **Jobs**: work that is running now. Neutral progress, not a warning.
 * - **Results**: what finished this session, newest first, bounded to
 *   `MAX_RESULTS`. Acknowledging one removes it.
 *
 * Entries hold stable codes and typed parameters rather than finished
 * sentences, so the center renders them in the current language and privacy
 * mode (see `notificationProjection.ts`). Actions are typed data and carry
 * their originating book scope; the center revalidates it before acting.
 *
 * Senders still on the old `addNotification` path land in the legacy list in
 * `store/ui.ts`, which the center shows under Recent.
 */

export type NotificationSeverity = "info" | "warning" | "error";
export type NotificationPriority = "blocking" | "attention" | "result";

/**
 * Where an entry came from. `book` is `bookIdentityKey(identity)` at the
 * moment the work started or the event arrived (null for app-wide entries);
 * `source` names a freshness source within that book. RAM only, never shown.
 */
export interface NotificationScope {
  book: string | null;
  source?: string;
}

export const CONDITION_CODES = [
  "journals.stale",
  "review.pending",
  "source.refresh-failed",
  "freshness.worker-error",
  "book-refresh.failed",
  "book-refresh.backend-required",
  "journals.process-failed",
  "connection.sync-failed",
  "btcpay.provenance-incomplete",
] as const;
export type ConditionCode = (typeof CONDITION_CODES)[number];

export const RESULT_CODES = [
  "book-refresh.finished",
  "book-refresh.pending",
  "book-refresh.problems",
  "journals.processed",
  "connection.synced",
] as const;
export type ResultCode = (typeof RESULT_CODES)[number];

export const JOB_CODES = [
  "book-refresh",
  "journal-processing",
  "connection-sync",
] as const;
export type JobCode = (typeof JOB_CODES)[number];

/**
 * An operation a notification may repeat, with the options it first ran
 * with. A closed list: anything else is refused when the action resolves.
 */
export type NotificationOperation =
  | { kind: "refresh-book"; forceFull: boolean }
  | { kind: "process-journals"; autoPair: boolean };

export const NOTIFICATION_OPERATION_KINDS = [
  "refresh-book",
  "process-journals",
] as const;

/**
 * One button. `navigate` opens a route from the allowlist in
 * `notificationRouting.ts`, optionally revealing one settings row (`anchor`,
 * a `SettingsAnchorId` of that section). `open-connection` and
 * `open-transaction` open one record of the originating book; from another
 * book they open the list instead. `operation` repeats a closed-list
 * operation and names `review`, the route a stale action opens instead of
 * mutating.
 */
export type NotificationActionSpec =
  | { type: "navigate"; to: string; anchor?: string }
  | { type: "open-connection"; connectionId: string }
  | { type: "open-transaction"; transactionId: string }
  | { type: "operation"; operation: NotificationOperation; review: string };

/**
 * Parameters a message may interpolate. Counts are of things (sources,
 * decisions, transactions), never money. `sources` carries user labels,
 * which only Normal and Hide amounts show.
 */
export interface NotificationParams {
  count?: number;
  sources?: string[];
  forceFull?: boolean;
  blocking?: boolean;
  transactions?: number;
  entries?: number;
  quarantined?: number;
  autoPaired?: number;
  warnings?: number;
}

/**
 * An expanded detail line. `label` is a user label (a source name);
 * `message` is operational text from the daemon, already scrubbed of
 * credentials there, bounded and scrubbed again here. Neither survives Hide
 * all; Hide amounts withholds a message that contains digits.
 */
export interface NotificationDetail {
  label?: string;
  message?: string;
  /** A fixed, translated phrase from a closed set; safe in every mode. */
  phraseKey?: NotificationPhraseKey;
}

export const NOTIFICATION_PHRASE_KEYS = [
  "failed",
  "cancelled",
  "coolingDown",
  "blocksReports",
  "autoPairSkipped",
] as const;
export type NotificationPhraseKey = (typeof NOTIFICATION_PHRASE_KEYS)[number];

interface EntryBase {
  id: string;
  scope: NotificationScope;
  severity: NotificationSeverity;
  priority: NotificationPriority;
  firstOccurredAt: string;
  lastOccurredAt: string;
  occurrenceCount: number;
  seenAt?: string;
  params: NotificationParams;
  action?: NotificationActionSpec;
  details?: NotificationDetail[];
}

export interface ConditionEntry extends EntryBase {
  kind: "condition";
  code: ConditionCode;
}

export interface ResultEntry extends EntryBase {
  kind: "job-result";
  code: ResultCode;
}

export interface JobProgress {
  value?: number;
  indeterminate?: boolean;
  /**
   * What the job is doing. `phaseKey` is a translated phase from a closed
   * set; `label` is producer text that may name a source, so Hide all drops
   * it.
   */
  phaseKey?: string;
  label?: string;
}

export interface JobEntry {
  id: string;
  code: JobCode;
  scope: NotificationScope;
  startedAt: string;
  updatedAt: string;
  params: NotificationParams;
  progress: JobProgress;
}

/** Recent holds this many finished results. Conditions are never counted. */
export const MAX_RESULTS = 30;
/** Detail lines per entry and characters per line; the rest is dropped. */
export const MAX_DETAILS = 12;
export const MAX_DETAIL_CHARS = 240;

export interface ConditionInput {
  code: ConditionCode;
  scope: NotificationScope;
  severity: NotificationSeverity;
  priority: Exclude<NotificationPriority, "result">;
  params?: NotificationParams;
  action?: NotificationActionSpec;
  details?: NotificationDetail[];
}

export interface ResultInput {
  code: ResultCode;
  scope: NotificationScope;
  severity: NotificationSeverity;
  params?: NotificationParams;
  action?: NotificationActionSpec;
  details?: NotificationDetail[];
}

export interface NotificationState {
  conditions: Record<string, ConditionEntry>;
  jobs: Record<string, JobEntry>;
  results: ResultEntry[];
  /** A new occurrence of a condition: it counts up. */
  raiseCondition: (input: ConditionInput) => void;
  /**
   * Bring conditions in line with an authoritative snapshot without counting
   * a new occurrence: `present` are created or refreshed, `resolved` keys go.
   * Anything the snapshot says nothing about stays as it is.
   */
  reconcileConditions: (update: {
    present?: ConditionInput[];
    resolved?: string[];
  }) => void;
  resolveCondition: (key: string) => void;
  startJob: (job: {
    id: string;
    code: JobCode;
    scope: NotificationScope;
    params?: NotificationParams;
    progress: JobProgress;
  }) => void;
  updateJob: (id: string, progress: JobProgress) => void;
  endJob: (id: string) => void;
  addResult: (input: ResultInput) => void;
  dismissResult: (id: string) => void;
  /** Clears results only. Conditions and running jobs stay. */
  clearResults: () => void;
  /** Opening the center marks what it shows as seen; it resolves nothing. */
  markSeen: () => void;
}

/** A condition's identity: scope + code + source. */
export function conditionKey(
  code: ConditionCode,
  scope: NotificationScope,
): string {
  return JSON.stringify([scope.book ?? null, code, scope.source ?? null]);
}

let sequence = 0;
function nextId(prefix: string): string {
  sequence += 1;
  return `${prefix}-${Date.now().toString(36)}-${sequence}`;
}

/** Bound and trim detail lines; empty lines go. */
export function boundDetails(
  details: readonly NotificationDetail[] | undefined,
): NotificationDetail[] | undefined {
  if (!details?.length) return undefined;
  const bounded = details
    .map((detail) => ({
      ...(detail.label ? { label: clip(detail.label) } : {}),
      ...(detail.message ? { message: clip(detail.message) } : {}),
      ...(detail.phraseKey &&
      (NOTIFICATION_PHRASE_KEYS as readonly string[]).includes(detail.phraseKey)
        ? { phraseKey: detail.phraseKey }
        : {}),
    }))
    .filter((detail) => detail.label || detail.message || detail.phraseKey)
    .slice(0, MAX_DETAILS);
  return bounded.length ? bounded : undefined;
}

function clip(text: string): string {
  const trimmed = text.replace(/\s+/g, " ").trim();
  return trimmed.length > MAX_DETAIL_CHARS
    ? `${trimmed.slice(0, MAX_DETAIL_CHARS - 1)}…`
    : trimmed;
}

function conditionFrom(
  input: ConditionInput,
  existing: ConditionEntry | undefined,
  now: string,
  countOccurrence: boolean,
): ConditionEntry {
  return {
    id: existing?.id ?? nextId("condition"),
    kind: "condition",
    code: input.code,
    scope: { ...input.scope },
    severity: input.severity,
    priority: input.priority,
    firstOccurredAt: existing?.firstOccurredAt ?? now,
    lastOccurredAt:
      countOccurrence || !existing ? now : existing.lastOccurredAt,
    occurrenceCount: existing
      ? existing.occurrenceCount + (countOccurrence ? 1 : 0)
      : 1,
    // A new occurrence is news again; a reconcile is not.
    seenAt: countOccurrence ? undefined : existing?.seenAt,
    params: { ...(input.params ?? {}) },
    action: input.action,
    details: boundDetails(input.details),
  };
}

function sameCondition(a: ConditionEntry, b: ConditionEntry): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export const useNotificationStore = create<NotificationState>()((set) => ({
  conditions: {},
  jobs: {},
  results: [],
  raiseCondition: (input) =>
    set((state) => {
      const key = conditionKey(input.code, input.scope);
      const now = new Date().toISOString();
      return {
        conditions: {
          ...state.conditions,
          [key]: conditionFrom(input, state.conditions[key], now, true),
        },
      };
    }),
  reconcileConditions: ({ present = [], resolved = [] }) =>
    set((state) => {
      const now = new Date().toISOString();
      let changed = false;
      const next = { ...state.conditions };
      for (const key of resolved) {
        if (key in next) {
          delete next[key];
          changed = true;
        }
      }
      for (const input of present) {
        const key = conditionKey(input.code, input.scope);
        const existing = next[key];
        const updated = conditionFrom(input, existing, now, false);
        if (existing && sameCondition(existing, updated)) continue;
        next[key] = updated;
        changed = true;
      }
      return changed ? { conditions: next } : state;
    }),
  resolveCondition: (key) =>
    set((state) => {
      if (!(key in state.conditions)) return state;
      const next = { ...state.conditions };
      delete next[key];
      return { conditions: next };
    }),
  startJob: ({ id, code, scope, params, progress }) =>
    set((state) => {
      const now = new Date().toISOString();
      return {
        jobs: {
          ...state.jobs,
          [id]: {
            id,
            code,
            scope: { ...scope },
            startedAt: now,
            updatedAt: now,
            params: { ...(params ?? {}) },
            progress,
          },
        },
      };
    }),
  updateJob: (id, progress) =>
    set((state) => {
      const job = state.jobs[id];
      if (!job) return state;
      return {
        jobs: {
          ...state.jobs,
          [id]: { ...job, progress, updatedAt: new Date().toISOString() },
        },
      };
    }),
  endJob: (id) =>
    set((state) => {
      if (!(id in state.jobs)) return state;
      const next = { ...state.jobs };
      delete next[id];
      return { jobs: next };
    }),
  addResult: (input) =>
    set((state) => {
      const now = new Date().toISOString();
      const entry: ResultEntry = {
        id: nextId("result"),
        kind: "job-result",
        code: input.code,
        scope: { ...input.scope },
        severity: input.severity,
        priority: "result",
        firstOccurredAt: now,
        lastOccurredAt: now,
        occurrenceCount: 1,
        params: { ...(input.params ?? {}) },
        action: input.action,
        details: boundDetails(input.details),
      };
      // One result per job outcome, book, and source: a repeat replaces the
      // earlier row rather than filling Recent.
      const rest = state.results.filter(
        (item) =>
          !(
            item.code === input.code &&
            item.scope.book === input.scope.book &&
            item.scope.source === input.scope.source
          ),
      );
      return { results: [entry, ...rest].slice(0, MAX_RESULTS) };
    }),
  dismissResult: (id) =>
    set((state) => ({
      results: state.results.filter((item) => item.id !== id),
    })),
  clearResults: () => set({ results: [] }),
  markSeen: () =>
    set((state) => {
      const now = new Date().toISOString();
      let changed = false;
      const conditions = Object.fromEntries(
        Object.entries(state.conditions).map(([key, entry]) => {
          if (entry.seenAt) return [key, entry];
          changed = true;
          return [key, { ...entry, seenAt: now }];
        }),
      );
      const results = state.results.map((entry) => {
        if (entry.seenAt) return entry;
        changed = true;
        return { ...entry, seenAt: now };
      });
      return changed ? { conditions, results } : state;
    }),
}));

/** Conditions that belong to the open book (or to no book), blockers first. */
export function conditionsInScope(
  conditions: Record<string, ConditionEntry>,
  book: string | null,
): ConditionEntry[] {
  return Object.values(conditions)
    .filter((entry) => entry.scope.book === null || entry.scope.book === book)
    .sort(compareConditions);
}

const PRIORITY_ORDER: Record<NotificationPriority, number> = {
  blocking: 0,
  attention: 1,
  result: 2,
};
const SEVERITY_ORDER: Record<NotificationSeverity, number> = {
  error: 0,
  warning: 1,
  info: 2,
};
// Live journal and review state leads its priority band: it is what the
// user can act on right away.
const CODE_ORDER: Record<ConditionCode, number> = {
  "journals.stale": 0,
  "review.pending": 1,
  "book-refresh.failed": 2,
  "journals.process-failed": 3,
  "book-refresh.backend-required": 4,
  "source.refresh-failed": 5,
  "freshness.worker-error": 6,
  "connection.sync-failed": 7,
  "btcpay.provenance-incomplete": 8,
};

function compareConditions(a: ConditionEntry, b: ConditionEntry): number {
  return (
    PRIORITY_ORDER[a.priority] - PRIORITY_ORDER[b.priority] ||
    CODE_ORDER[a.code] - CODE_ORDER[b.code] ||
    SEVERITY_ORDER[a.severity] - SEVERITY_ORDER[b.severity] ||
    a.firstOccurredAt.localeCompare(b.firstOccurredAt)
  );
}

/**
 * One row per code: per-source conditions with the same code fold into one
 * group, which is what the badge counts.
 */
export interface ConditionGroup {
  key: string;
  code: ConditionCode;
  entries: ConditionEntry[];
  severity: NotificationSeverity;
  priority: Exclude<NotificationPriority, "result">;
}

export function groupConditions(
  conditions: readonly ConditionEntry[],
): ConditionGroup[] {
  const groups = new Map<string, ConditionEntry[]>();
  for (const entry of conditions) {
    const key = JSON.stringify([entry.scope.book ?? null, entry.code]);
    groups.set(key, [...(groups.get(key) ?? []), entry]);
  }
  return [...groups.entries()]
    .map(([key, entries]) => {
      const sorted = [...entries].sort(compareConditions);
      return {
        key,
        code: sorted[0].code,
        entries: sorted,
        severity: sorted.reduce<NotificationSeverity>(
          (worst, entry) =>
            SEVERITY_ORDER[entry.severity] < SEVERITY_ORDER[worst]
              ? entry.severity
              : worst,
          "info",
        ),
        priority: sorted.some((entry) => entry.priority === "blocking")
          ? ("blocking" as const)
          : ("attention" as const),
      };
    })
    .sort(
      (a, b) =>
        PRIORITY_ORDER[a.priority] - PRIORITY_ORDER[b.priority] ||
        CODE_ORDER[a.code] - CODE_ORDER[b.code],
    );
}

/** The badge: unresolved attention groups for the open book. */
export function attentionGroupCount(
  conditions: Record<string, ConditionEntry>,
  book: string | null,
): number {
  return groupConditions(conditionsInScope(conditions, book)).length;
}

/** Empties the center: when the local session resets, and in tests. */
export function resetNotificationStore(): void {
  useNotificationStore.setState({ conditions: {}, jobs: {}, results: [] });
}
