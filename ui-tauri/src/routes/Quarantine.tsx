import {
  QuarantineDashboard,
  QuarantineUnavailable,
  type QuarantineSnapshot,
} from "@/components/kb/quarantine";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { useDaemon } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";

const QUARANTINE_LIMIT = 100;

export function Quarantine() {
  const { data, isLoading, isError, error } = useDaemon<QuarantineSnapshot>(
    "ui.journals.quarantine",
    { limit: QUARANTINE_LIMIT },
  );
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction();

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

  const snapshot = data.data;

  return (
    <QuarantineDashboard
      snapshot={snapshot}
      isProcessingJournals={isProcessingJournals}
      onProcessJournals={runJournalProcessing}
    />
  );
}
