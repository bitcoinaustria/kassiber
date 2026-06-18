import { useTranslation } from "react-i18next";

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
  const { t } = useTranslation("journals");
  const rows = quarantineRows(snapshot, t);
  const metrics = quarantineMetrics(snapshot.summary, t);
  const reasonGroupCount = snapshot.summary.by_reason.length;

  return (
    <ReviewDataTable
      kind="quarantine"
      eyebrow={t("quarantine.eyebrow")}
      title={t("quarantine.title")}
      description={t("quarantine.description")}
      rows={rows}
      metrics={metrics}
      showSummaryBadge={false}
      badgeLabel={
        snapshot.summary.count
          ? t("quarantine.badge.quarantined", {
              count: snapshot.summary.count,
            })
          : t("quarantine.badge.clear")
      }
      tableTitle={t("quarantine.tableTitle")}
      tableDescription={t("quarantine.tableDescription", {
        count: reasonGroupCount,
        rows: rows.length,
      })}
      searchPlaceholder={t("quarantine.searchPlaceholder")}
      emptyMessage={t("quarantine.empty")}
      actions={
        <QuarantineActions
          isProcessingJournals={isProcessingJournals}
          onProcessJournals={onProcessJournals}
        />
      }
    />
  );
}
