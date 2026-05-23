import { ReviewDataTable } from "@/components/kb/ReviewDataTable";

import { quarantineMetrics, quarantineRows } from "./model";
import { QuarantineActions } from "./QuarantineActions";
import type { QuarantineSnapshot } from "./types";

interface QuarantineDashboardProps {
  snapshot: QuarantineSnapshot;
  isProcessingJournals: boolean;
  onProcessJournals: () => void;
}

export function QuarantineDashboard({
  snapshot,
  isProcessingJournals,
  onProcessJournals,
}: QuarantineDashboardProps) {
  const rows = quarantineRows(snapshot);
  const metrics = quarantineMetrics(snapshot.summary);

  return (
    <ReviewDataTable
      kind="quarantine"
      eyebrow="Review queue"
      title="Quarantine"
      description="Blocked transactions held out of journals and reports until missing prices, basis, assets, or pair evidence are fixed."
      rows={rows}
      metrics={metrics}
      showSummaryBadge={false}
      badgeLabel={
        snapshot.summary.count
          ? `${snapshot.summary.count.toLocaleString("en-US")} quarantined`
          : "clear"
      }
      tableTitle="Quarantined transactions"
      tableDescription={`${rows.length.toLocaleString("en-US")} shown · ${snapshot.summary.by_reason.length.toLocaleString("en-US")} reason group${snapshot.summary.by_reason.length === 1 ? "" : "s"}`}
      searchPlaceholder="Search wallet, txid, reason, amount..."
      emptyMessage="No quarantined transactions in the active book."
      actions={
        <QuarantineActions
          isProcessingJournals={isProcessingJournals}
          onProcessJournals={onProcessJournals}
        />
      }
    />
  );
}
