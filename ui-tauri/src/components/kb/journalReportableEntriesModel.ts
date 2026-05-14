import type { ReviewMetric } from "@/components/kb/ReviewDataTable";

export interface JournalEventTypeSummary {
  type: string;
  count: number;
  gainLossEur: number;
}

export interface ReportableJournalEntryMetricInput {
  entryType: string;
  gainLossEur: number | null;
}

export interface ReportableJournalEntrySummary {
  [key: string]: unknown;
  needsJournals: boolean;
  entryTypes: JournalEventTypeSummary[];
}

export function reportableEntryMetrics(
  summary: ReportableJournalEntrySummary,
): ReviewMetric[] {
  const acquisitions = countEntryTypes(summary.entryTypes, "acquisition");
  const disposals = countEntryTypes(summary.entryTypes, "disposal");
  const income = countEntryTypes(summary.entryTypes, "income");
  const fees = countEntryTypes(summary.entryTypes, "fee", "transfer_fee");
  const neutral = countEntryTypes(
    summary.entryTypes,
    "neutral_swap",
    "transfer_in",
    "transfer_out",
  );
  return [
    {
      label: "Acquisitions",
      value: acquisitions,
      tone: summary.needsJournals
        ? "warning"
        : acquisitions
          ? "good"
          : "neutral",
      filterId: "acquisitions",
    },
    {
      label: "Disposals",
      value: disposals,
      tone: disposals ? "warning" : "neutral",
      filterId: "disposals",
    },
    {
      label: "Income",
      value: income,
      tone: income ? "good" : "neutral",
      filterId: "income",
    },
    {
      label: "Fees",
      value: fees,
      tone: fees ? "alert" : "neutral",
      filterId: "fees",
    },
    {
      label: "Neutral",
      value: neutral,
      tone: neutral ? "good" : "neutral",
      filterId: "neutral",
      filterLabel: "Neutral entries",
    },
  ];
}

export function reportableEntryMetricFilterIds(
  event: ReportableJournalEntryMetricInput,
) {
  const filters: string[] = [];
  if (event.entryType === "acquisition") {
    filters.push("acquisitions");
  }
  if (event.entryType === "disposal") {
    filters.push("disposals");
  }
  if (event.entryType === "income") {
    filters.push("income");
  }
  if (["fee", "transfer_fee"].includes(event.entryType)) {
    filters.push("fees");
  }
  if (["neutral_swap", "transfer_in", "transfer_out"].includes(event.entryType)) {
    filters.push("neutral");
  }
  return filters;
}

function countEntryTypes(
  entries: JournalEventTypeSummary[],
  ...types: string[]
) {
  return entries.reduce(
    (total, row) => (types.includes(row.type) ? total + row.count : total),
    0,
  );
}
