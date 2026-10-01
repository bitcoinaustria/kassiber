import * as React from "react";

import {
  QuarantineDashboard,
  QuarantineUnavailable,
  type QuarantineScope,
  type QuarantineSnapshot,
} from "@/components/kb/quarantine";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { normalizeQuarantineSnapshot } from "@/lib/normalizeUiSnapshots";

// The daemon serves at most this many rows per request; larger scopes page
// through the same causes-first ordering.
const QUARANTINE_PAGE_SIZE = 100;

export function Quarantine() {
  const [scope, setScope] = React.useState<QuarantineScope>("attention");
  const [offset, setOffset] = React.useState(0);
  // What needs the user is always read: it drives the summary and the causes.
  const attentionQuery = useDaemon<QuarantineSnapshot>("ui.journals.quarantine", {
    limit: QUARANTINE_PAGE_SIZE,
    offset: scope === "attention" ? offset : 0,
    scope: "attention",
  });
  const listQuery = useDaemon<QuarantineSnapshot>(
    "ui.journals.quarantine",
    { limit: QUARANTINE_PAGE_SIZE, offset, scope },
    { enabled: scope !== "attention" },
  );
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction();
  const { data, isLoading, isError, error } = attentionQuery;
  const attention = React.useMemo(
    () => (data?.data ? normalizeQuarantineSnapshot(data.data) : null),
    [data?.data],
  );
  const listed = React.useMemo(
    () =>
      scope === "attention"
        ? attention
        : listQuery.data?.data
          ? normalizeQuarantineSnapshot(listQuery.data.data)
          : null,
    [attention, listQuery.data?.data, scope],
  );
  const scopeTotal = listed?.summary.scope_count ?? listed?.summary.count ?? 0;

  React.useEffect(() => {
    // A rebuild can shrink the scope below the current page.
    if (offset > 0 && scopeTotal > 0 && offset >= scopeTotal) setOffset(0);
  }, [offset, scopeTotal]);

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
      list={listed}
      listLoading={scope !== "attention" && listQuery.isLoading}
      scope={scope}
      onScopeChange={(next) => {
        setScope(next);
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
