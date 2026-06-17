import { Loader2, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";

import {
  ReviewDataTable,
  type ReviewTableRow,
} from "@/components/kb/ReviewDataTable";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Button } from "@/components/ui/button";
import { useDaemon } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { screenPanelClassName } from "@/lib/screen-layout";
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
  const { t } = useTranslation("journals");
  const { data, isLoading, isError, error } = useDaemon<JournalEventsSnapshot>(
    "ui.journals.events.list",
    { limit: REPORTABLE_ENTRY_LIMIT },
  );
  const { runJournalProcessing, isProcessingJournals } =
    useJournalProcessingAction();

  if (isLoading) {
    return <ScreenSkeleton titleWidth="w-52" />;
  }

  if (isError || data?.error || !data?.data) {
    return (
      <div className={screenPanelClassName}>
        <div className="rounded-xl border bg-card p-4">
          <h2 className="text-base font-semibold">
            {t("reportable.unavailable.title")}
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {error instanceof Error
              ? error.message
              : data?.error?.message ?? t("reportable.unavailable.fallback")}
          </p>
        </div>
      </div>
    );
  }

  const snapshot = data.data;
  const rows = snapshot.events.map((event) =>
    reportableEntryToRow(event, snapshot.summary, t),
  );
  const metrics = reportableEntryMetrics(snapshot.summary);

  return (
    <ReviewDataTable
      kind="journal-events"
      eyebrow={t("reportable.eyebrow")}
      title={t("reportable.title")}
      description={t("reportable.description")}
      rows={rows}
      metrics={metrics}
      showSummaryBadge={false}
      showStateColumn={false}
      showPriorityBadge={false}
      badgeLabel={
        snapshot.summary.needsJournals
          ? t("reportable.badge.stale")
          : t("reportable.badge.entries", { count: snapshot.summary.count })
      }
      shellClassName="w-full space-y-3 sm:space-y-4"
      tableTitle={t("reportable.tableTitle")}
      tableDescriptionDetail={snapshot.summary.freshnessReason}
      searchPlaceholder={t("reportable.searchPlaceholder")}
      emptyMessage={t("reportable.empty")}
      actions={
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
          {t("reportable.actions.processJournals")}
        </Button>
      }
    />
  );
}

function reportableEntryToRow(
  event: ReportableJournalEntry,
  summary: JournalEventsSnapshot["summary"],
  t: TFunction<"journals">,
): ReviewTableRow {
  const atLabel =
    event.atKennzahl !== null
      ? t("reportable.atKennzahl", { kennzahl: event.atKennzahl })
      : event.atCategory
        ? localizedEntryType(event.atCategory, t)
        : "";
  const status: ReviewTableRow["status"] = summary.needsJournals
    ? "Needs review"
    : "Ready";
  return {
    id: shortId(event.transactionExternalId || event.transactionId || event.id),
    date: formatDate(event.occurredAt || event.createdAt, t),
    account: event.accountLabel || event.account || event.wallet,
    event: eventTitle(event, t),
    source: [event.wallet, atLabel].filter(Boolean).join(" · "),
    amount: formatMsatAmount(event.quantityMsat, event.asset),
    basis: basisText(event, t),
    impact:
      event.gainLossEur === null ? eur.format(0) : eur.format(event.gainLossEur),
    status,
    priority: status === "Ready" ? "Low" : "Medium",
    owner: summary.profile ?? t("quarantine.ownerFallback"),
    evidenceHint: eventEvidenceHint(event, summary, t),
    nextAction: summary.needsJournals
      ? t("reportable.nextAction.needsJournals")
      : event.atKennzahl !== null
        ? t("reportable.nextAction.austrianMapping")
        : t("reportable.nextAction.reports"),
    metricFilterIds: reportableEntryMetricFilterIds(event),
  };
}

function eventTitle(event: ReportableJournalEntry, t: TFunction<"journals">) {
  const typeLabel = localizedEntryType(event.entryType, t);
  const description = stripJournalMarkers(event.description).trim();
  return description
    ? t("reportable.eventTitle", { type: typeLabel, description })
    : typeLabel;
}

// Daemon entry-type CODES map to translated labels; unknown codes fall back to
// a structural humanizer of the code itself (the code is the stable id).
const ENTRY_TYPE_LABEL_KEYS: Record<string, string> = {
  acquisition: "ledger.entryType.acquisition",
  disposal: "ledger.entryType.disposal",
  income: "ledger.entryType.income",
  fee: "ledger.entryType.fee",
  transfer_fee: "ledger.entryType.transferFee",
  transfer: "ledger.entryType.transfer",
  transfer_in: "ledger.entryType.transferIn",
  transfer_out: "ledger.entryType.transferOut",
  neutral_swap: "ledger.entryType.neutralSwap",
};

function localizedEntryType(type: string, t: TFunction<"journals">) {
  const key = ENTRY_TYPE_LABEL_KEYS[type];
  return key ? t(key) : formatEntryType(type);
}

function basisText(event: ReportableJournalEntry, t: TFunction<"journals">) {
  if (event.costBasisEur !== null || event.proceedsEur !== null) {
    return [
      event.costBasisEur !== null
        ? t("reportable.basis.amount", { value: eur.format(event.costBasisEur) })
        : null,
      event.proceedsEur !== null
        ? t("reportable.basis.proceeds", {
            value: eur.format(event.proceedsEur),
          })
        : null,
    ]
      .filter(Boolean)
      .join(" · ");
  }
  if (event.fiatValueEur)
    return t("reportable.basis.value", {
      value: eur.format(event.fiatValueEur),
    });
  return t("reportable.basis.noFiatImpact");
}

function pricingSourceLabel(
  event: ReportableJournalEntry,
  t: TFunction<"journals">,
) {
  const source = event.pricingSourceKind;
  const quality = event.pricingQuality;
  if (!source && !quality) return t("reportable.pricing.label");
  if (source === "manual_override") return t("reportable.pricing.manual");
  if (source === "source_price") return t("reportable.pricing.source");
  if (source === "rate_cache") return t("reportable.pricing.cached");
  if (source === "fmv_provider") {
    return quality === "provider_sample"
      ? t("reportable.pricing.providerSample")
      : t("reportable.pricing.providerPrice");
  }
  const sourceLabel = formatEntryType(source || "pricing");
  return quality
    ? t("reportable.pricing.qualified", {
        source: sourceLabel,
        quality: formatEntryType(quality),
      })
    : sourceLabel;
}

// Kept separate so eventEvidenceHint can stay readable while preserving compact row copy.
function eventEvidenceHint(
  event: ReportableJournalEntry,
  summary: JournalEventsSnapshot["summary"],
  t: TFunction<"journals">,
) {
  if (summary.needsJournals) return summary.freshnessReason;
  if (event.atKennzahl !== null)
    return t("reportable.evidence.austrianMapping", {
      kennzahl: event.atKennzahl,
    });
  if (event.pricingSourceKind)
    return t("reportable.evidence.pricedBy", {
      source: pricingSourceLabel(event, t),
    });
  if (event.entryType === "transfer_in" || event.entryType === "transfer_out") {
    return t("reportable.evidence.transferMatching");
  }
  return t("reportable.evidence.journalProcessing");
}

function stripJournalMarkers(description: string) {
  return description
    .split(/\s+/)
    .filter(
      (token) =>
        !token.startsWith("at_regime=") &&
        !token.startsWith("at_pool=") &&
        !token.startsWith("at_swap_link="),
    )
    .join(" ");
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

function formatDate(value: string, t: TFunction<"journals">) {
  return value ? value.slice(0, 10) : t("review.unknownDate");
}

function shortId(value: string) {
  return value.length > 12 ? `${value.slice(0, 10)}...` : value;
}
