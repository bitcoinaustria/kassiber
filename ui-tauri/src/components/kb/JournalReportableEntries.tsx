import { CircleDollarSign, Loader2, RefreshCw } from "lucide-react";

import {
  ReviewDataTable,
  type ReviewTableRow,
} from "@/components/kb/ReviewDataTable";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useDaemon } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { screenPanelClassName } from "@/lib/screen-layout";
import { useUiStore } from "@/store/ui";
import {
  reportableEntryMetricFilterIds,
  reportableEntryMetrics,
  type JournalEventTypeSummary,
} from "./journalReportableEntriesModel";

interface ReportableJournalEntry {
  id: string;
  transactionId: string;
  transactionExternalId: string;
  transactionDirection: string;
  occurredAt: string;
  createdAt: string;
  entryType: string;
  wallet: string;
  account: string;
  accountLabel: string;
  asset: string;
  quantity: number;
  quantityMsat: number;
  fiatValueEur: number;
  unitCostEur: number;
  costBasisEur: number | null;
  proceedsEur: number | null;
  gainLossEur: number | null;
  pricingSourceKind: string;
  pricingQuality: string;
  description: string;
  atCategory: string | null;
  atKennzahl: number | null;
}

interface JournalEventsSnapshot {
  summary: {
    workspace: string | null;
    profile: string | null;
    count: number;
    reportableCount: number;
    needsJournals: boolean;
    lastProcessedAt: string | null;
    freshnessStatus: string;
    freshnessReason: string;
    entryTypes: JournalEventTypeSummary[];
    limit: number;
  };
  events: ReportableJournalEntry[];
}

const REPORTABLE_ENTRY_LIMIT = 500;
const eur = new Intl.NumberFormat("de-AT", {
  style: "currency",
  currency: "EUR",
});

export function JournalReportableEntries() {
  const { data, isLoading, isError, error } = useDaemon<JournalEventsSnapshot>(
    "ui.journals.events.list",
    { limit: REPORTABLE_ENTRY_LIMIT },
  );
  const dataMode = useUiStore((s) => s.dataMode);
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction();

  if (isLoading) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading reportable entries...
      </div>
    );
  }

  if (isError || data?.error || !data?.data) {
    return (
      <div className={screenPanelClassName}>
        <div className="rounded-xl border bg-card p-4">
          <h2 className="text-base font-semibold">
            Reportable entries unavailable
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {error instanceof Error
              ? error.message
              : data?.error?.message ??
                "The daemon did not return reportable journal entries."}
          </p>
        </div>
      </div>
    );
  }

  const snapshot = data.data;
  const rows = snapshot.events.map((event) =>
    reportableEntryToRow(event, snapshot.summary),
  );
  const metrics = reportableEntryMetrics(snapshot.summary);

  return (
    <ReviewDataTable
      kind="journal-events"
      eyebrow="Review · journal ledger"
      title="Reportable Entries"
      description="Processed journal entries with tax classification, pricing provenance, basis, proceeds, and gain/loss. This is the detailed report-readiness view."
      icon={CircleDollarSign}
      rows={rows}
      metrics={metrics}
      showSummaryBadge={false}
      badgeLabel={
        snapshot.summary.needsJournals
          ? "stale"
          : `${snapshot.summary.count.toLocaleString("en-US")} entries`
      }
      shellClassName="w-full space-y-3 sm:space-y-4"
      tableTitle="Reportable journal entries"
      tableDescriptionDetail={snapshot.summary.freshnessReason}
      searchPlaceholder="Search wallet, entry, asset, pricing, Kennzahl..."
      emptyMessage="No processed journal entries yet. Process journals after importing transactions."
      actions={
        <>
          {dataMode === "mock" ? (
            <Badge variant="outline" className="rounded-md">
              Preview data
            </Badge>
          ) : null}
          <Button
            type="button"
            className="h-9"
            onClick={runJournalProcessing}
            disabled={isProcessingJournals}
          >
            {isProcessingJournals ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <RefreshCw className="size-4" aria-hidden="true" />
            )}
            Process journals
          </Button>
        </>
      }
    />
  );
}

function reportableEntryToRow(
  event: ReportableJournalEntry,
  summary: JournalEventsSnapshot["summary"],
): ReviewTableRow {
  const pricingLabel = pricingSourceLabel(event);
  const atLabel =
    event.atKennzahl !== null
      ? `AT Kennzahl ${event.atKennzahl}`
      : event.atCategory
        ? formatEntryType(event.atCategory)
        : "";
  const status: ReviewTableRow["status"] = summary.needsJournals
    ? "Needs review"
    : "Ready";
  return {
    id: shortId(event.transactionExternalId || event.transactionId || event.id),
    date: formatDate(event.occurredAt || event.createdAt),
    account: event.accountLabel || event.account || event.wallet,
    event: eventTitle(event),
    source: [event.wallet, pricingLabel, atLabel].filter(Boolean).join(" · "),
    amount: formatMsatAmount(event.quantityMsat, event.asset),
    basis: basisText(event),
    impact:
      event.gainLossEur === null ? eur.format(0) : eur.format(event.gainLossEur),
    status,
    priority: status === "Ready" ? "Low" : "Medium",
    owner: summary.profile ?? "Active book",
    evidenceHint: eventEvidenceHint(event, summary),
    nextAction: summary.needsJournals
      ? "Process journals before relying on this entry"
      : event.atKennzahl !== null
        ? "Ready for Austrian report mapping"
        : "Ready for reports",
    metricFilterIds: reportableEntryMetricFilterIds(event),
  };
}

function eventTitle(event: ReportableJournalEntry) {
  const typeLabel = formatEntryType(event.entryType);
  const description = event.description.trim();
  return description ? `${typeLabel} · ${description}` : typeLabel;
}

function basisText(event: ReportableJournalEntry) {
  if (event.costBasisEur !== null || event.proceedsEur !== null) {
    return [
      event.costBasisEur !== null ? `Basis ${eur.format(event.costBasisEur)}` : null,
      event.proceedsEur !== null ? `Proceeds ${eur.format(event.proceedsEur)}` : null,
    ]
      .filter(Boolean)
      .join(" · ");
  }
  if (event.fiatValueEur) return `Value ${eur.format(event.fiatValueEur)}`;
  return "No fiat impact";
}

function pricingSourceLabel(event: ReportableJournalEntry) {
  const source = event.pricingSourceKind || "pricing";
  const quality = event.pricingQuality ? `/${event.pricingQuality}` : "";
  return `${source}${quality}`;
}

function eventEvidenceHint(
  event: ReportableJournalEntry,
  summary: JournalEventsSnapshot["summary"],
) {
  if (summary.needsJournals) return summary.freshnessReason;
  if (event.atKennzahl !== null) return `Austrian form mapping ${event.atKennzahl}`;
  if (event.pricingSourceKind) return `Priced by ${pricingSourceLabel(event)}`;
  if (event.entryType === "transfer_in" || event.entryType === "transfer_out") {
    return "Carried by transfer matching";
  }
  return "Computed by journal processing";
}

function formatEntryType(type: string) {
  return type
    .replaceAll("_", " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function formatMsatAmount(msat: number, asset: string) {
  const sats = Math.round(Math.abs(msat) / 1000);
  const sign = msat < 0 ? "-" : "";
  return `${sign}${sats.toLocaleString("en-US")} sats ${asset}`.trim();
}

function formatDate(value: string) {
  return value ? value.slice(0, 10) : "Unknown";
}

function shortId(value: string) {
  return value.length > 12 ? `${value.slice(0, 10)}...` : value;
}
