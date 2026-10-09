import * as React from "react";

import {
  QuarantineDashboard,
  QuarantineUnavailable,
  type QuarantineRefreshed,
  type QuarantineSnapshot,
} from "@/components/kb/quarantine";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { normalizeQuarantineSnapshot } from "@/lib/normalizeUiSnapshots";

// The daemon serves at most this many rows per request; the waiting list
// pages through the rest in the daemon's order.
const QUARANTINE_PAGE_SIZE = 100;

export function Quarantine() {
  const [waitingShown, setWaitingShown] = React.useState(false);
  const [offset, setOffset] = React.useState(0);
  // What needs the user is always read: it drives the summary and the causes.
  const attentionQuery = useDaemon<QuarantineSnapshot>("ui.journals.quarantine", {
    limit: QUARANTINE_PAGE_SIZE,
    offset: 0,
    scope: "attention",
  });
  // What only waits on a cause is read once the owner asks for it.
  const waitingQuery = useDaemon<QuarantineSnapshot>(
    "ui.journals.quarantine",
    { limit: QUARANTINE_PAGE_SIZE, offset, scope: "waiting" },
    { enabled: waitingShown },
  );
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction();
  const { data, isLoading, isError, error } = attentionQuery;
  const attention = React.useMemo(
    () => (data?.data ? normalizeQuarantineSnapshot(data.data) : null),
    [data?.data],
  );
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
    if (freshAttention.isError || !freshAttention.data?.data || freshAttention.data.error) {
      return null;
    }
    const attentionPage = normalizeQuarantineSnapshot(freshAttention.data.data);
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

  if (isError || data?.error || !attention) {
    return (
      <QuarantineUnavailable
        message={error instanceof Error ? error.message : data?.error?.message}
      />
    );
  }

  return (
    <QuarantineDashboard
      attention={attention}
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
