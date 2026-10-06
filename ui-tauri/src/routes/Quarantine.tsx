import * as React from "react";

import {
  QuarantineDashboard,
  QuarantineUnavailable,
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
      waitingShown={waitingShown}
      onWaitingShownChange={(shown) => {
        setWaitingShown(shown);
        setOffset(0);
      }}
      offset={offset}
      pageSize={QUARANTINE_PAGE_SIZE}
      onOffsetChange={setOffset}
      isProcessingJournals={isProcessingJournals}
      onProcessJournals={runJournalProcessing}
    />
  );
}
