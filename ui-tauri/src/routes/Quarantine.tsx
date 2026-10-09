import * as React from "react";

import {
  QuarantineDashboard,
  QuarantineUnavailable,
  type QuarantineRefreshed,
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
  // A failed Waiting or All page is an error, never an empty queue.
  const listFailed =
    scope !== "attention" && (listQuery.isError || Boolean(listQuery.data?.error));
  const listError = listFailed
    ? (listQuery.error instanceof Error
        ? listQuery.error.message
        : listQuery.data?.error?.message) ?? ""
    : null;

  const { refetch: refetchAttention } = attentionQuery;
  const { refetch: refetchList } = listQuery;
  // Re-reads exactly the pages this route shows, for this book and scope; a
  // failed read yields nothing rather than the pages from before.
  const refresh = React.useCallback(async (): Promise<QuarantineRefreshed | null> => {
    const [freshAttention, freshList] = await Promise.all([
      refetchAttention(),
      scope === "attention" ? Promise.resolve(null) : refetchList(),
    ]);
    if (freshAttention.isError || !freshAttention.data?.data || freshAttention.data.error) {
      return null;
    }
    const attentionPage = normalizeQuarantineSnapshot(freshAttention.data.data);
    if (freshList === null) return { attention: attentionPage, list: attentionPage };
    if (freshList.isError || !freshList.data?.data || freshList.data.error) return null;
    return { attention: attentionPage, list: normalizeQuarantineSnapshot(freshList.data.data) };
  }, [refetchAttention, refetchList, scope]);

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
      listError={listError}
      onRetryList={() => void listQuery.refetch()}
      scope={scope}
      onScopeChange={(next) => {
        setScope(next);
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
