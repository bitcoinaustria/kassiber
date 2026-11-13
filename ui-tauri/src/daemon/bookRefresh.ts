/**
 * Book refresh cycles: how the desktop brings every mounted read up to a new
 * state of the book at once.
 *
 * A sync, a journal run, or a background freshness event changes the book.
 * Refetching each query on its own lets the screen update piece by piece, so
 * for a moment the Overview could show the new balance beside the old
 * transaction list. A cycle instead:
 *
 * 1. holds every mounted `useDaemon` result at the data it already shows
 *    (see `useHeldQueryResult`), so nothing on screen blanks or changes yet;
 * 2. reads the book's revision (`status` with `revision: true`), refetches the
 *    invalidated queries, then reads the revision again, and repeats the
 *    refetch when a write landed in between (a worker committed), up to
 *    `MAX_ATTEMPTS` times;
 * 3. releases the hold, so every screen swaps to the new data in one render,
 *    and records what changed (new transactions, the balance move) for the
 *    title bar's freshness indicator and the row highlights.
 *
 * Only reads the user already has on screen are refetched, as before; the
 * revision read is local and never leaves the machine.
 */
import { create } from "zustand";
import {
  hashKey,
  partialMatchKey,
  type InfiniteData,
  type Query,
  type QueryClient,
  type QueryKey,
} from "@tanstack/react-query";
import { useRef } from "react";

import type { DaemonEnvelope } from "./transport";

export const MAX_ATTEMPTS = 3;
/** A cycle never holds the screen longer than this, even if reads hang. */
export const HOLD_LIMIT_MS = 15_000;
/** How long new rows stay highlighted after a cycle lands them. */
export const HIGHLIGHT_MS = 6_000;
/** How long the title bar keeps naming what the last refresh changed. */
export const CHANGE_SUMMARY_MS = 30_000;

export interface BookChange {
  /** Change in the book's active transaction count (negative when removed). */
  transactionDelta: number;
  /** Change in the displayed total balance, in sats; null when unknown. */
  balanceDeltaSat: number | null;
  at: number;
}

interface BookRefreshState {
  /** Mounted reads keep showing their committed data while true. */
  holding: boolean;
  /** A cycle is refetching (the hold may already have timed out). */
  cycling: boolean;
  lastChange: BookChange | null;
  /** Transaction ids a cycle just brought onto the screen. */
  highlightedTxIds: ReadonlySet<string>;
  dismissChange: () => void;
}

const EMPTY_IDS: ReadonlySet<string> = new Set();

export const useBookRefreshStore = create<BookRefreshState>()((set) => ({
  holding: false,
  cycling: false,
  lastChange: null,
  highlightedTxIds: EMPTY_IDS,
  dismissChange: () => set({ lastChange: null }),
}));

type QueryPredicate = (query: Query) => boolean;

export interface RefreshRequest {
  /** The query-key prefix to refresh, e.g. `["daemon", dataMode]`. */
  queryKey: QueryKey;
  /** Narrows the refresh to some kinds; omitted refreshes every query. */
  predicate?: QueryPredicate;
}

export interface BookRefreshDeps {
  /** The open book's revision token, or null when it cannot be read. */
  readRevision: () => Promise<string | null>;
  /** The query keys whose data names the book's balance and transactions. */
  overviewKey: () => QueryKey | null;
  now?: () => number;
}

let running: Promise<void> | null = null;
let queued: RefreshRequest | null = null;
let queuedWaiters: Array<() => void> = [];
let highlightTimer: ReturnType<typeof setTimeout> | undefined;
let summaryTimer: ReturnType<typeof setTimeout> | undefined;

function mergeRequests(a: RefreshRequest, b: RefreshRequest): RefreshRequest {
  if (hashKey(a.queryKey) !== hashKey(b.queryKey)) {
    // Different prefixes (another data mode): refresh both prefixes' queries.
    const prefixes = [a, b];
    return {
      queryKey: [],
      predicate: (query) =>
        prefixes.some(
          (request) =>
            partialMatchKey(query.queryKey, request.queryKey) &&
            (!request.predicate || request.predicate(query)),
        ),
    };
  }
  if (!a.predicate || !b.predicate) return { queryKey: a.queryKey };
  const left = a.predicate;
  const right = b.predicate;
  return { queryKey: a.queryKey, predicate: (q) => left(q) || right(q) };
}

/**
 * Refresh the matching queries as one cycle. A request that arrives while a
 * cycle runs is merged into one follow-up cycle; the returned promise
 * settles once the cycle that covers this request has landed.
 */
export function refreshBookQueries(
  queryClient: QueryClient,
  request: RefreshRequest,
  deps: BookRefreshDeps,
): Promise<void> {
  // Mark the matching queries stale at once, so a screen mounted before the
  // cycle reaches it still refetches; the cycle itself does the refetching.
  void queryClient
    .invalidateQueries({
      queryKey: request.queryKey,
      predicate: request.predicate,
      refetchType: "none",
    })
    .catch(() => undefined);
  if (running) {
    queued = queued ? mergeRequests(queued, request) : request;
    return new Promise((resolve) => queuedWaiters.push(resolve));
  }
  running = runCycles(queryClient, request, deps);
  return running;
}

async function runCycles(
  queryClient: QueryClient,
  first: RefreshRequest,
  deps: BookRefreshDeps,
): Promise<void> {
  let request: RefreshRequest | null = first;
  try {
    while (request) {
      await runCycle(queryClient, request, deps);
      request = queued;
      const waiters = queuedWaiters;
      queued = null;
      queuedWaiters = [];
      if (!request) waiters.forEach((resolve) => resolve());
      else queuedWaiters = waiters;
    }
  } finally {
    running = null;
    queued = null;
    const waiters = queuedWaiters;
    queuedWaiters = [];
    waiters.forEach((resolve) => resolve());
  }
}

async function runCycle(
  queryClient: QueryClient,
  request: RefreshRequest,
  deps: BookRefreshDeps,
): Promise<void> {
  const now = deps.now ?? Date.now;
  // Fixed for the whole cycle: a book or profile switch mid-cycle must not
  // compare one book's figures with another's.
  const overviewKey = deps.overviewKey();
  let before: BookView | null = null;
  useBookRefreshStore.setState({ holding: true, cycling: true });
  const holdLimit = setTimeout(() => {
    useBookRefreshStore.setState({ holding: false });
  }, HOLD_LIMIT_MS);
  try {
    before = captureBookView(queryClient, overviewKey);
    // The overview carries the balance and count the change summary names.
    // When it is cached but not mounted and this request marked it stale,
    // it takes part in every attempt: its first refetch clears the flag,
    // and a retry must not keep the older copy.
    const refreshOverview =
      overviewKey !== null &&
      queryClient.getQueryState(overviewKey)?.isInvalidated === true;
    for (let attempt = 1; attempt <= MAX_ATTEMPTS; attempt += 1) {
      const start = await deps.readRevision();
      await queryClient
        .refetchQueries(
          { queryKey: request.queryKey, predicate: request.predicate, type: "active" },
          { cancelRefetch: true },
        )
        .catch(() => undefined);
      if (refreshOverview && overviewKey) {
        await queryClient
          .refetchQueries({ queryKey: overviewKey, exact: true, type: "inactive" })
          .catch(() => undefined);
      }
      const end = await deps.readRevision();
      if (start === null || end === null || start === end) break;
    }
  } catch {
    // Each read keeps its own error state; the cycle still has to end.
  } finally {
    // Release first: nothing below may leave the screens held.
    clearTimeout(holdLimit);
    useBookRefreshStore.setState({ holding: false, cycling: false });
  }
  try {
    const current = deps.overviewKey();
    const sameBook =
      before !== null &&
      (overviewKey === null
        ? current === null
        : current !== null && hashKey(current) === hashKey(overviewKey));
    if (before && sameBook) publishChange(before, captureBookView(queryClient, overviewKey), now());
  } catch {
    // The summary is a courtesy; a malformed payload must not break the cycle.
  }
}

function publishChange(before: BookView, after: BookView, at: number) {
  const change = describeChange(before, after, at);
  // Rows "new" to a list whose book did not gain transactions are rows that
  // moved up (one was removed), not arrivals.
  const gained =
    before.transactionCount === null ||
    after.transactionCount === null ||
    after.transactionCount > before.transactionCount;
  const newIds = gained ? newTransactionIds(before, after) : new Set<string>();
  if (!change && newIds.size === 0) return;
  useBookRefreshStore.setState((state) => ({
    lastChange: change ?? state.lastChange,
    highlightedTxIds: newIds.size > 0 ? newIds : state.highlightedTxIds,
  }));
  if (newIds.size > 0) {
    clearTimeout(highlightTimer);
    highlightTimer = setTimeout(
      () => useBookRefreshStore.setState({ highlightedTxIds: EMPTY_IDS }),
      HIGHLIGHT_MS,
    );
  }
  if (change) {
    clearTimeout(summaryTimer);
    summaryTimer = setTimeout(() => {
      if (useBookRefreshStore.getState().lastChange?.at === change.at) {
        useBookRefreshStore.setState({ lastChange: null });
      }
    }, CHANGE_SUMMARY_MS);
  }
}

/** Forget the last change and highlights (another book, profile, or mode). */
export function clearBookChange() {
  clearTimeout(highlightTimer);
  clearTimeout(summaryTimer);
  useBookRefreshStore.setState({ lastChange: null, highlightedTxIds: EMPTY_IDS });
}

/** What a refresh can compare: the overview's figures and the visible rows. */
export interface BookView {
  transactionCount: number | null;
  balanceSat: number | null;
  /** Visible transaction ids per cached query (hashed key), page by page. */
  rows: Map<string, string[][]>;
}

interface OverviewLike {
  status?: { transactionCount?: number };
  balanceSummary?: { totalBtc?: number };
  txs?: Array<{ id?: unknown }>;
}

interface TransactionsLike {
  txs?: Array<{ id?: unknown }>;
}

function ids(rows: unknown): string[] {
  if (!Array.isArray(rows)) return [];
  return rows
    .map((row: unknown) =>
      row && typeof row === "object" ? (row as { id?: unknown }).id : undefined,
    )
    .filter((id): id is string => typeof id === "string" && id.length > 0);
}

export function captureBookView(
  queryClient: QueryClient,
  overviewKey: QueryKey | null,
): BookView {
  const overview = overviewKey
    ? queryClient.getQueryData<DaemonEnvelope<OverviewLike>>(overviewKey)?.data
    : undefined;
  const rows = new Map<string, string[][]>();
  for (const query of queryClient.getQueryCache().getAll()) {
    const key = query.queryKey;
    if (key[0] !== "daemon") continue;
    const data = query.state.data;
    if (key.includes("ui.overview.snapshot")) {
      const envelope = data as DaemonEnvelope<OverviewLike> | undefined;
      rows.set(query.queryHash, [ids(envelope?.data?.txs)]);
    } else if (key.includes("ui.transactions.list")) {
      const pages = (data as InfiniteData<DaemonEnvelope<TransactionsLike>> | undefined)
        ?.pages;
      if (!Array.isArray(pages)) continue;
      rows.set(
        query.queryHash,
        pages.map((page) => ids(page?.data?.txs)),
      );
    }
  }
  const count = overview?.status?.transactionCount;
  const total = overview?.balanceSummary?.totalBtc;
  return {
    transactionCount: typeof count === "number" && Number.isFinite(count) ? count : null,
    // Display figures arrive as BTC floats; sats are exact enough to name a
    // move and never feed accounting.
    balanceSat:
      typeof total === "number" && Number.isFinite(total)
        ? Math.round(total * 1e8)
        : null,
    rows,
  };
}

export function describeChange(
  before: BookView,
  after: BookView,
  at: number,
): BookChange | null {
  const transactionDelta =
    before.transactionCount !== null && after.transactionCount !== null
      ? after.transactionCount - before.transactionCount
      : 0;
  const balanceDeltaSat =
    before.balanceSat !== null && after.balanceSat !== null
      ? after.balanceSat - before.balanceSat
      : null;
  if (transactionDelta === 0 && !balanceDeltaSat) return null;
  return { transactionDelta, balanceDeltaSat: balanceDeltaSat || null, at };
}

/** Ids that a cached list shows now and did not show before the cycle. */
export function newTransactionIds(before: BookView, after: BookView): Set<string> {
  const fresh = new Set<string>();
  for (const [hash, pages] of after.rows) {
    const earlierPages = before.rows.get(hash);
    // A list that was not loaded before has nothing to compare with.
    if (!earlierPages) continue;
    const earlier = new Set(earlierPages.flat());
    // Only as many pages as were loaded before: a page loaded meanwhile
    // (scrolling) holds older rows, not new ones.
    for (const id of pages.slice(0, earlierPages.length).flat()) {
      if (!earlier.has(id)) fresh.add(id);
    }
  }
  return fresh;
}

/**
 * The query result as the screen should render it: during a cycle, the data
 * it showed when the cycle began; otherwise the live result, unchanged.
 *
 * A different query key is a different dataset (another filter, period, or
 * wallet), so it is never held at the previous key's data.
 */
export function useHeldQueryResult<R extends { data: unknown }>(
  queryKey: QueryKey,
  result: R,
): R {
  const holding = useBookRefreshStore((state) => state.holding);
  const keyHash = hashKey(queryKey);
  const live = result.data;
  // Placeholder data is another key's (`keepPreviousData`); it is never the
  // copy a hold keeps, or the old dataset would stay up as this key's.
  const placeholder =
    (result as { isPlaceholderData?: unknown }).isPlaceholderData === true;
  const committed = useRef<{ key: string; data: unknown } | null>(null);
  const shown = committed.current;
  if (
    !holding ||
    shown === null ||
    shown.key !== keyHash ||
    shown.data === undefined
  ) {
    committed.current = { key: keyHash, data: placeholder ? undefined : live };
    return result;
  }
  if (shown.data === live) return result;
  return withHeldData(result, shown.data);
}

/** The live result with the held data; it still counts as refreshing. */
function withHeldData<R extends object>(result: R, data: unknown): R {
  const live = result as Record<string, unknown>;
  const held = {} as R;
  for (const key of Object.keys(result)) {
    Object.defineProperty(held, key, {
      enumerable: true,
      get: () => {
        if (key === "data") return data;
        // Newer data is waiting behind the hold: the screen is refreshing,
        // and what it shows is this key's own earlier data.
        if (key === "isFetching") return true;
        if (key === "isPlaceholderData") return false;
        return live[key];
      },
    });
  }
  return held;
}

/** Test hook: forget queued work and timers between tests. */
export function resetBookRefreshForTests() {
  running = null;
  queued = null;
  queuedWaiters = [];
  clearTimeout(highlightTimer);
  clearTimeout(summaryTimer);
  useBookRefreshStore.setState({
    holding: false,
    cycling: false,
    lastChange: null,
    highlightedTxIds: EMPTY_IDS,
  });
}
