import * as React from "react";
import { useTranslation } from "react-i18next";

import {
  QuarantineDashboard,
  QuarantineUnavailable,
  type QuarantineSnapshot,
} from "@/components/kb/quarantine";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Button } from "@/components/ui/button";
import { useDaemon } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { normalizeQuarantineSnapshot } from "@/lib/normalizeUiSnapshots";

// The daemon serves at most this many rows per request; larger quarantines
// page through the same root-first ordering.
const QUARANTINE_PAGE_SIZE = 100;

export function Quarantine() {
  const { t } = useTranslation("journals");
  const [offset, setOffset] = React.useState(0);
  const { data, isLoading, isError, error } = useDaemon<QuarantineSnapshot>(
    "ui.journals.quarantine",
    { limit: QUARANTINE_PAGE_SIZE, offset },
  );
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction();
  const total = data?.data?.summary?.count ?? 0;

  React.useEffect(() => {
    // A rebuild can shrink the quarantine below the current page.
    if (offset > 0 && total > 0 && offset >= total) setOffset(0);
  }, [offset, total]);

  if (isLoading) {
    return <ScreenSkeleton titleWidth="w-40" />;
  }

  if (isError || data?.error || !data?.data) {
    return (
      <QuarantineUnavailable
        message={
          error instanceof Error ? error.message : data?.error?.message
        }
      />
    );
  }

  const snapshot = normalizeQuarantineSnapshot(data.data);
  const pageEnd = Math.min(offset + snapshot.items.length, snapshot.summary.count);

  return (
    <>
      <QuarantineDashboard
        snapshot={snapshot}
        isProcessingJournals={isProcessingJournals}
        onProcessJournals={runJournalProcessing}
      />
      {snapshot.summary.count > QUARANTINE_PAGE_SIZE ? (
        <nav
          className="mt-3 flex items-center justify-end gap-2 text-xs text-muted-foreground"
          aria-label={t("quarantine.tableTitle")}
        >
          <span className="tabular-nums">
            {t("quarantine.paging.range", {
              from: snapshot.items.length ? offset + 1 : 0,
              to: pageEnd,
              total: snapshot.summary.count,
            })}
          </span>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={offset === 0}
            onClick={() => setOffset(Math.max(0, offset - QUARANTINE_PAGE_SIZE))}
          >
            {t("quarantine.paging.previous")}
          </Button>
          <Button
            type="button"
            size="sm"
            variant="outline"
            disabled={pageEnd >= snapshot.summary.count}
            onClick={() => setOffset(offset + QUARANTINE_PAGE_SIZE)}
          >
            {t("quarantine.paging.next")}
          </Button>
        </nav>
      ) : null}
    </>
  );
}
