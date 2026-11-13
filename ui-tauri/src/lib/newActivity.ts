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
    id: string;
    direction: "incoming" | "outgoing" | null;
    wallet?: string;
  }>;
  /** New rows a source reported only as a count. */
  unlisted: Array<{ wallet?: string; count: number }>;
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
) {
  const wallet = text(outcome.wallet) ?? fallbackLabel;
  const records = outcome.inserted_records;
  if (Array.isArray(records)) {
    for (const record of records) {
      if (!isRecord(record)) continue;
      const id = text(record.transaction_id);
      if (!id || seen.has(id)) continue;
      seen.add(id);
      activity.records.push({
        id,
        direction:
          record.direction === "inbound"
            ? "incoming"
            : record.direction === "outbound"
              ? "outgoing"
              : null,
        wallet: text(record.wallet) ?? wallet,
      });
    }
    return;
  }
  const imported = outcome.imported;
  if (typeof imported === "number" && Number.isSafeInteger(imported) && imported > 0) {
    activity.unlisted.push({ wallet, count: imported });
  }
}

/**
 * What a run (a foreground book refresh or a background event) added: the
 * per-wallet results it returned and the freshness jobs it completed. A
 * transaction listed twice counts once. Null when nothing is new.
 */
export function newActivityFromRun(
  data: FreshnessRunData | null | undefined,
): NewActivity | null {
  if (!data) return null;
  const activity: NewActivity = { records: [], unlisted: [] };
  const seen = new Set<string>();
  for (const result of data.results ?? []) {
    if (!isRecord(result) || result.status !== "synced") continue;
    collectOutcome(activity, seen, result, undefined);
  }
  for (const job of data.completed ?? []) {
    if (job.status !== "done" || !isRecord(job.result)) continue;
    collectOutcome(activity, seen, job.result, job.source_label || undefined);
  }
  return activity.records.length || activity.unlisted.length ? activity : null;
}

/**
 * File (or grow) the book's "new transactions" entry. Until the center has
 * shown it, later runs add to the same entry, so an afternoon of background
 * syncs is one row rather than one per run. Once seen, the next run starts
 * a fresh entry.
 */
export function recordNewActivity(
  activity: NewActivity | null,
  scope: NotificationScope,
): void {
  if (!activity) return;
  const store = useNotificationStore.getState();
  const earlier = store.results.find(
    (entry) =>
      entry.code === "activity.new" &&
      entry.scope.book === scope.book &&
      !entry.seenAt,
  );
  const known = new Set(earlier?.params.transactionIds ?? []);
  const fresh = activity.records.filter((record) => !known.has(record.id));
  const unlistedCount = activity.unlisted.reduce((sum, item) => sum + item.count, 0);
  if (fresh.length === 0 && unlistedCount === 0) return;

  const wallets = new Map<string, number>();
  for (const detail of earlier?.details ?? []) {
    if (detail.label && detail.newTransactions) {
      wallets.set(detail.label, detail.newTransactions);
    }
  }
  const addWallet = (label: string | undefined, count: number) => {
    if (label) wallets.set(label, (wallets.get(label) ?? 0) + count);
  };
  for (const record of fresh) addWallet(record.wallet, 1);
  for (const item of activity.unlisted) addWallet(item.wallet, item.count);

  const transactionIds = [...known, ...fresh.map((record) => record.id)];
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
    scope: { book: scope.book, expected: scope.expected },
    severity: "info",
    params: {
      transactions: total,
      incoming,
      outgoing,
      transactionIds,
      ...(walletList.length === 1 ? { sources: [walletList[0][0]] } : {}),
    },
    action,
    // Kept even for one wallet, so a later run can merge per wallet; the
    // center lists them only when there are several.
    details: walletList.map(([label, count]) => ({ label, newTransactions: count })),
  });
}
