import { Link } from "@tanstack/react-router";
import {
  BookOpen,
  CircleDollarSign,
  FileText,
  Loader2,
  RefreshCw,
} from "lucide-react";

import {
  ReviewDataTable,
  type ReviewMetric,
  type ReviewTableRow,
} from "@/components/kb/ReviewDataTable";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useDaemon, useDaemonMutation } from "@/daemon/client";
import { screenPanelClassName } from "@/lib/screen-layout";
import { useUiStore } from "@/store/ui";

interface JournalEventTypeSummary {
  type: string;
  count: number;
  gainLossEur: number;
}

interface JournalTaxEvent {
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
  events: JournalTaxEvent[];
}

interface JournalProcessResult {
  entries_created?: number;
  processed_transactions?: number;
  quarantined?: number;
}

const TAX_EVENT_LIMIT = 100;
const eur = new Intl.NumberFormat("de-AT", {
  style: "currency",
  currency: "EUR",
});

export function TaxEvents() {
  const { data, isLoading, isError, error } = useDaemon<JournalEventsSnapshot>(
    "ui.journals.events.list",
    { limit: TAX_EVENT_LIMIT },
  );
  const dataMode = useUiStore((s) => s.dataMode);
  const addNotification = useUiStore((s) => s.addNotification);
  const processJournals =
    useDaemonMutation<JournalProcessResult>("ui.journals.process");

  const runJournalProcessing = () => {
    if (processJournals.isPending) return;
    processJournals.mutate(undefined, {
      onSuccess: (envelope) => {
        const payload = envelope.data;
        addNotification({
          title: "Journals processed",
          body: [
            payload?.processed_transactions !== undefined
              ? `${payload.processed_transactions} transactions`
              : null,
            payload?.entries_created !== undefined
              ? `${payload.entries_created} events`
              : null,
            payload?.quarantined !== undefined
              ? `${payload.quarantined} quarantined`
              : null,
          ]
            .filter(Boolean)
            .join(", "),
          tone: payload?.quarantined ? "warning" : "success",
        });
      },
      onError: (mutationError) => {
        addNotification({
          title: "Journal processing failed",
          body:
            mutationError instanceof Error
              ? mutationError.message
              : "Could not process journals.",
          tone: "error",
        });
      },
    });
  };

  if (isLoading) {
    return (
      <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
        Loading tax events...
      </div>
    );
  }

  if (isError || data?.error || !data?.data) {
    return (
      <div className={screenPanelClassName}>
        <div className="rounded-xl border bg-card p-4">
          <h2 className="text-base font-semibold">Tax events unavailable</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {error instanceof Error
              ? error.message
              : data?.error?.message ??
                "The daemon did not return tax event data."}
          </p>
        </div>
      </div>
    );
  }

  const snapshot = data.data;
  const rows = snapshot.events.map((event) =>
    taxEventToRow(event, snapshot.summary),
  );
  const metrics = taxEventMetrics(snapshot.summary);

  return (
    <ReviewDataTable
      kind="tax-events"
      eyebrow="Review · tax event ledger"
      title="Tax Events"
      description="Computed journal events with tax classification, pricing provenance, basis, proceeds, and gain/loss. This is where processed tax meaning is inspected before reports are exported."
      icon={CircleDollarSign}
      rows={rows}
      metrics={metrics}
      showSummaryBadge={false}
      badgeLabel={
        snapshot.summary.needsJournals
          ? "stale"
          : `${snapshot.summary.count.toLocaleString("en-US")} events`
      }
      tableTitle="Processed tax events"
      tableDescription={`${rows.length.toLocaleString("en-US")} shown · ${snapshot.summary.freshnessReason}`}
      searchPlaceholder="Search wallet, event, asset, pricing, Kennzahl..."
      emptyMessage="No processed tax events yet. Process journals after importing transactions."
      actions={
        <>
          {dataMode === "mock" ? (
            <Badge variant="outline" className="rounded-md">
              Preview data
            </Badge>
          ) : null}
          <Button asChild variant="outline" className="h-9">
            <Link to="/journals">
              <BookOpen className="size-4" aria-hidden="true" />
              Journals
            </Link>
          </Button>
          <Button asChild variant="outline" className="h-9">
            <Link to="/reports">
              <FileText className="size-4" aria-hidden="true" />
              Reports
            </Link>
          </Button>
          <Button
            type="button"
            className="h-9"
            onClick={runJournalProcessing}
            disabled={processJournals.isPending}
          >
            {processJournals.isPending ? (
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

function taxEventToRow(
  event: JournalTaxEvent,
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
      ? "Process journals before relying on this event"
      : event.atKennzahl !== null
        ? "Ready for Austrian report mapping"
        : "Ready for reports",
  };
}

function taxEventMetrics(
  summary: JournalEventsSnapshot["summary"],
): ReviewMetric[] {
  const disposals = countEntryTypes(summary.entryTypes, "disposal");
  const incomeAndFees = countEntryTypes(
    summary.entryTypes,
    "income",
    "fee",
    "transfer_fee",
  );
  const gainLoss = summary.entryTypes.reduce(
    (total, row) => total + row.gainLossEur,
    0,
  );
  return [
    {
      label: "Events",
      value: summary.count,
      tone: summary.needsJournals ? "warning" : summary.count ? "good" : "neutral",
    },
    {
      label: "Reportable",
      value: summary.reportableCount,
      tone: summary.reportableCount ? "good" : "neutral",
    },
    {
      label: "Disposals",
      value: disposals,
      tone: disposals ? "warning" : "neutral",
    },
    {
      label: incomeAndFees ? "Income/fees" : "Net gain",
      value: incomeAndFees || eur.format(gainLoss),
      tone: gainLoss < 0 ? "alert" : gainLoss > 0 ? "good" : "neutral",
    },
  ];
}

function eventTitle(event: JournalTaxEvent) {
  const typeLabel = formatEntryType(event.entryType);
  const description = event.description.trim();
  return description ? `${typeLabel} · ${description}` : typeLabel;
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

function basisText(event: JournalTaxEvent) {
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

function pricingSourceLabel(event: JournalTaxEvent) {
  const source = event.pricingSourceKind || "pricing";
  const quality = event.pricingQuality ? `/${event.pricingQuality}` : "";
  return `${source}${quality}`;
}

function eventEvidenceHint(
  event: JournalTaxEvent,
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
