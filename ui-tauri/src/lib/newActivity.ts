import type { FreshnessRunData } from "@/lib/syncResults";
import {
  useNotificationStore,
  type NotificationActionSpec,
  type NotificationScope,
} from "@/store/notifications";

/**
 * New transactions a sync run committed, for the center's "new
 * transactions" entry. Directions and ids only: amounts stay on the
 * Transactions page, where the privacy mode and the ledger's own fee
 * handling apply.
 */
export interface NewActivity {
  /** Transactions the daemon listed, each once. */
  records: Array<{
    /** The row id, for opening one transaction. */
    id: string;
    /**
     * What identifies the transaction across runs, scoped like the daemon's
     * own fingerprint: external id (the txid), wallet, direction, and asset
     * when the txid is known, so a rescan that re-inserts a row under a new
     * id is not announced again while both sides of a transfer between own
     * wallets still are; otherwise the row id.
     */
    key: string;
    /** The freshness job that reported it, when known. */
    job?: string;
    direction: "incoming" | "outgoing" | null;
    wallet?: string;
  }>;
  /** New rows a source reported only as a count. */
  unlisted: Array<{
    wallet?: string;
    count: number;
    /** The freshness job that reported it, when known; replays are skipped. */
    job?: string;
  }>;
}

/** Wallets a "new transactions" entry keeps counts for; a sane upper bound. */
const MAX_WALLETS = 500;

/**
 * What this session already announced, per book: transaction keys and the
 * jobs behind count-only outcomes, so a replayed event does not report them
 * again once their entry has been seen. RAM only, bounded.
 */
const MAX_ANNOUNCED = 5_000;
const announced = new Set<string>();
/**
 * Completed jobs already announced, kept apart from the per-transaction
 * keys so evicting old keys never lets a replayed job through.
 */
const MAX_ANNOUNCED_JOBS = 2_000;
const announcedJobs = new Set<string>();

function rememberJob(book: string | null, job: string) {
  announcedJobs.add(announcedKey(book, job));
  if (announcedJobs.size > MAX_ANNOUNCED_JOBS) {
    const oldest = announcedJobs.values().next().value;
    if (oldest !== undefined) announcedJobs.delete(oldest);
  }
}

function announcedKey(book: string | null, key: string): string {
  return JSON.stringify([book, key]);
}

function remember(book: string | null, key: string) {
  announced.add(announcedKey(book, key));
  if (announced.size > MAX_ANNOUNCED) {
    const oldest = announced.values().next().value;
    if (oldest !== undefined) announced.delete(oldest);
  }
}

/** For tests: forget what this session announced. */
export function resetAnnouncedActivity(): void {
  announced.clear();
  announcedJobs.clear();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function text(value: unknown): string | undefined {
  return typeof value === "string" && value ? value : undefined;
}

/**
 * One wallet's sync outcome: `inserted_records` when the daemon lists them,
 * otherwise its `imported` count.
 */
function collectOutcome(
  activity: NewActivity,
  seen: Set<string>,
  outcome: Record<string, unknown>,
  fallbackLabel: string | undefined,
  job: string | undefined,
) {
  const wallet = text(outcome.wallet) ?? fallbackLabel;
  const records = outcome.inserted_records;
  if (Array.isArray(records)) {
    for (const record of records) {
      if (!isRecord(record)) continue;
      const id = text(record.transaction_id);
      if (!id) continue;
      const external = text(record.external_id);
      const recordWallet = text(record.wallet) ?? wallet;
      const key = external
        ? JSON.stringify([external, recordWallet ?? null, record.direction ?? null, record.asset ?? null])
        : id;
      if (seen.has(key)) continue;
      seen.add(key);
      activity.records.push({
        id,
        key,
        ...(job ? { job } : {}),
        direction:
          record.direction === "inbound"
            ? "incoming"
            : record.direction === "outbound"
              ? "outgoing"
              : null,
        wallet: recordWallet,
      });
    }
    return;
  }
  const imported = outcome.imported;
  if (typeof imported === "number" && Number.isSafeInteger(imported) && imported > 0) {
    activity.unlisted.push({ wallet, count: imported, ...(job ? { job } : {}) });
  }
}

/**
 * What a run (a foreground book refresh or a background event) added. A
 * run's `results` copy the outcomes of the wallet jobs it completed, but a
 * wallet synced directly appears only in `results` (next to, say, a
 * completed journal job). So every completed job is read, then each result
 * whose wallet no completed job already reported. A transaction listed
 * twice counts once. Null when nothing is new.
 */
export function newActivityFromRun(
  data: FreshnessRunData | null | undefined,
): NewActivity | null {
  if (!data) return null;
  const activity: NewActivity = { records: [], unlisted: [] };
  const seen = new Set<string>();
  const covered = new Set<string>();
  for (const job of data.completed ?? []) {
    if (job.status !== "done" || !isRecord(job.result)) continue;
    const label = text(job.result.wallet) ?? (job.source_label || undefined);
    if (label) covered.add(label);
    collectOutcome(activity, seen, job.result, job.source_label || undefined, job.id);
  }
  for (const result of data.results ?? []) {
    if (!isRecord(result) || result.status !== "synced") continue;
    const label = text(result.wallet);
    if (label && covered.has(label)) continue;
    collectOutcome(activity, seen, result, undefined, undefined);
  }
  return activity.records.length || activity.unlisted.length ? activity : null;
}

/**
 * File (or grow) the book's "new transactions" entry. Until the center has
 * shown it, later runs add to the same entry, so an afternoon of background
 * syncs is one row rather than one per run. Once seen, the next run starts
 * a fresh entry; anything this session already announced for the book is
 * not announced again.
 */
export function recordNewActivity(
  activity: NewActivity | null,
  scope: NotificationScope,
): void {
  if (!activity) return;
  const book = scope.book;
  const store = useNotificationStore.getState();
  const earlier = store.results.find(
    (entry) =>
      entry.code === "activity.new" && entry.scope.book === book && !entry.seenAt,
  );
  const jobSeen = (job: string | undefined) =>
    job !== undefined && announcedJobs.has(announcedKey(book, job));
  const fresh = activity.records.filter(
    (record) => !jobSeen(record.job) && !announced.has(announcedKey(book, record.key)),
  );
  const unlisted = activity.unlisted.filter((item) => !jobSeen(item.job));
  const unlistedCount = unlisted.reduce((sum, item) => sum + item.count, 0);
  if (fresh.length === 0 && unlistedCount === 0) return;
  for (const record of fresh) remember(book, record.key);
  for (const item of [...fresh, ...unlisted]) if (item.job) rememberJob(book, item.job);

  const wallets = new Map<string, number>();
  for (const wallet of earlier?.params.walletCounts ?? []) {
    wallets.set(wallet.label, wallet.count);
  }
  const addWallet = (label: string | undefined, count: number) => {
    if (label) wallets.set(label, (wallets.get(label) ?? 0) + count);
  };
  for (const record of fresh) addWallet(record.wallet, 1);
  for (const item of unlisted) addWallet(item.wallet, item.count);

  const transactionIds = [
    ...(earlier?.params.transactionIds ?? []),
    ...fresh.map((record) => record.id),
  ];
  const total = (earlier?.params.transactions ?? 0) + fresh.length + unlistedCount;
  const incoming =
    (earlier?.params.incoming ?? 0) +
    fresh.filter((record) => record.direction === "incoming").length;
  const outgoing =
    (earlier?.params.outgoing ?? 0) +
    fresh.filter((record) => record.direction === "outgoing").length;
  const only = total === 1 && transactionIds.length === 1 ? transactionIds[0] : null;
  const action: NotificationActionSpec = only
    ? { type: "open-transaction", transactionId: only }
    : { type: "navigate", to: "/transactions" };
  const walletList = [...wallets];

  store.addResult({
    code: "activity.new",
    scope: { book, expected: scope.expected },
    severity: "info",
    params: {
      transactions: total,
      incoming,
      outgoing,
      transactionIds,
      ...(walletList.length === 1 ? { sources: [walletList[0][0]] } : {}),
      // Whole, even for one wallet, so a later run merges per wallet; the
      // center shortens it for display.
      walletCounts: walletList
        .slice(0, MAX_WALLETS)
        .map(([label, count]) => ({ label, count })),
    },
    action,
  });
}
