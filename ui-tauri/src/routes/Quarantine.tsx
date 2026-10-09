import * as React from "react";

import {
  QuarantineDashboard,
  QuarantineUnavailable,
  type QuarantineRefreshed,
  type QuarantineSnapshot,
} from "@/components/kb/quarantine";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon, useDaemonInfinite } from "@/daemon/client";
import type { DaemonEnvelope } from "@/daemon/transport";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { normalizeQuarantineSnapshot } from "@/lib/normalizeUiSnapshots";

// The daemon serves at most this many rows per request; both lists page
// through the rest in the daemon's order.
const QUARANTINE_PAGE_SIZE = 100;

/** The next attention page's offset, or nothing once every row is loaded. */
function nextAttentionOffset(page: DaemonEnvelope<QuarantineSnapshot>) {
  const summary = page.data?.summary;
  if (!summary) return undefined;
  const next = (summary.offset ?? 0) + (page.data?.items.length ?? 0);
  const total = summary.scope_count ?? summary.count ?? 0;
  return page.data?.items.length && next < total ? next : undefined;
}

/**
 * The loaded attention pages as one snapshot: the whole-book summary of the
 * first page and every loaded row once, in the daemon's order.
 */
function mergeAttentionPages(
  pages: Array<DaemonEnvelope<QuarantineSnapshot>> | undefined,
): QuarantineSnapshot | null {
  const first = pages?.[0]?.data;
  if (!pages || !first) return null;
  const seen = new Set<string>();
  const items = pages
    .flatMap((page) => normalizeQuarantineSnapshot(page.data).items)
    .filter((item) => !seen.has(item.transaction_id) && Boolean(seen.add(item.transaction_id)));
  return { ...normalizeQuarantineSnapshot(first), items };
}

export function Quarantine() {
  const [waitingShown, setWaitingShown] = React.useState(false);
  const [offset, setOffset] = React.useState(0);
  // What needs the user is always read: it drives the summary and the causes.
  // More of it loads on request, so a cause with more rows than one page
  // holds can be worked through without resolving the first page first.
  const attentionQuery = useDaemonInfinite<QuarantineSnapshot>(
    "ui.journals.quarantine",
    { limit: QUARANTINE_PAGE_SIZE, offset: 0, scope: "attention" },
    nextAttentionOffset,
  );
  // What only waits on a cause is read once the owner asks for it.
  const waitingQuery = useDaemon<QuarantineSnapshot>(
    "ui.journals.quarantine",
    { limit: QUARANTINE_PAGE_SIZE, offset, scope: "waiting" },
    { enabled: waitingShown },
  );
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction();
  const { data, isLoading, error } = attentionQuery;
  const attention = React.useMemo(() => mergeAttentionPages(data?.pages), [data?.pages]);
  // A failed later page keeps what is loaded and says so where more loads.
  const moreFailed = attentionQuery.isFetchNextPageError;
  const waiting = React.useMemo(
    () =>
      waitingShown && waitingQuery.data?.data
        ? normalizeQuarantineSnapshot(waitingQuery.data.data)
        : null,
    [waitingQuery.data?.data, waitingShown],
  );
  const waitingTotal = waiting?.summary.scope_count ?? 0;
  // A failed waiting page is an error, never an empty list.
  const waitingFailed =
    waitingShown && (waitingQuery.isError || Boolean(waitingQuery.data?.error));
  const waitingError = waitingFailed
    ? (waitingQuery.error instanceof Error
        ? waitingQuery.error.message
        : waitingQuery.data?.error?.message) ?? ""
    : null;

  const { refetch: refetchAttention } = attentionQuery;
  const { refetch: refetchWaiting } = waitingQuery;
  // Re-reads exactly the pages this route shows, for this book; a failed
  // read yields nothing rather than the pages from before.
  const refresh = React.useCallback(async (): Promise<QuarantineRefreshed | null> => {
    const [freshAttention, freshWaiting] = await Promise.all([
      refetchAttention(),
      waitingShown ? refetchWaiting() : Promise.resolve(null),
    ]);
    const attentionPage = freshAttention.isError ? null : mergeAttentionPages(freshAttention.data?.pages);
    if (!attentionPage) return null;
    if (freshWaiting === null) return { attention: attentionPage, list: null };
    if (freshWaiting.isError || !freshWaiting.data?.data || freshWaiting.data.error) return null;
    return { attention: attentionPage, list: normalizeQuarantineSnapshot(freshWaiting.data.data) };
  }, [refetchAttention, refetchWaiting, waitingShown]);

  React.useEffect(() => {
    // A rebuild can shrink the list below the current page.
    if (offset > 0 && waitingTotal > 0 && offset >= waitingTotal) setOffset(0);
  }, [offset, waitingTotal]);

  if (isLoading) {
    return <ScreenSkeleton titleWidth="w-40" />;
  }

  if (!attention || (attentionQuery.isError && !moreFailed)) {
    return (
      <QuarantineUnavailable message={error instanceof Error ? error.message : undefined} />
    );
  }

  return (
    <QuarantineDashboard
      attention={attention}
      attentionMore={{
        hasMore: Boolean(attentionQuery.hasNextPage),
        loading: attentionQuery.isFetchingNextPage,
        error: moreFailed && error instanceof Error ? error.message : moreFailed ? "" : null,
        onLoad: () => void attentionQuery.fetchNextPage(),
      }}
      waiting={waiting}
      waitingLoading={waitingShown && waitingQuery.isLoading}
      waitingError={waitingError}
      onRetryWaiting={() => void waitingQuery.refetch()}
      waitingShown={waitingShown}
      onWaitingShownChange={(shown) => {
        setWaitingShown(shown);
        setOffset(0);
      }}
      offset={offset}
      pageSize={QUARANTINE_PAGE_SIZE}
      onOffsetChange={setOffset}
      onRefresh={refresh}
      isProcessingJournals={isProcessingJournals}
      onProcessJournals={runJournalProcessing}
    />
  );
}
