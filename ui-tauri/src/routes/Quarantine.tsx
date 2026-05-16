import { Link } from "@tanstack/react-router";
import { Loader2, RefreshCw, TableProperties } from "lucide-react";

import {
  ReviewDataTable,
  type ReviewMetric,
  type ReviewTableRow,
} from "@/components/kb/ReviewDataTable";
import { ScreenSkeleton } from "@/components/kb/ScreenSkeleton";
import { Button } from "@/components/ui/button";
import { useDaemon } from "@/daemon/client";
import { useJournalProcessingAction } from "@/hooks/useJournalProcessingAction";
import { screenPanelClassName } from "@/lib/screen-layout";

interface QuarantineReason {
  reason: string;
  count: number;
}

interface QuarantineItem {
  transaction_id: string;
  external_id: string;
  occurred_at: string;
  confirmed_at: string | null;
  wallet: string;
  direction: "inbound" | "outbound" | string;
  asset: string;
  amount: number;
  amount_msat: number;
  fee: number;
  fee_msat: number;
  reason: string;
  detail: Record<string, unknown>;
  created_at: string;
}

interface QuarantineSnapshot {
  summary: {
    workspace: string | null;
    profile: string | null;
    count: number;
    by_reason: QuarantineReason[];
    limit: number;
  };
  items: QuarantineItem[];
}

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
      <div className={screenPanelClassName}>
        <div className="rounded-xl border bg-card p-4">
          <h2 className="text-base font-semibold">Quarantine unavailable</h2>
          <p className="mt-1 text-sm text-muted-foreground">
            {error instanceof Error
              ? error.message
              : data?.error?.message ??
                "The daemon did not return quarantine data."}
          </p>
        </div>
      </div>
    );
  }

  const snapshot = data.data;
  const rows = snapshot.items.map((item) =>
    quarantineItemToRow(item, snapshot.summary.profile),
  );
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
        <>
          <Button asChild variant="outline" className="h-9">
            <Link to="/transactions">
              <TableProperties className="size-4" aria-hidden="true" />
              Transactions
            </Link>
          </Button>
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

function quarantineItemToRow(
  item: QuarantineItem,
  profile: string | null,
): ReviewTableRow {
  const reason = item.reason || "review_required";
  const reasonText = formatReason(reason);
  const priority = quarantinePriority(reason);
  const status = priority === "High" ? "Blocked" : "Needs review";
  const amountMsat =
    item.direction === "outbound" ? -Math.abs(item.amount_msat) : item.amount_msat;
  return {
    id: item.external_id || shortId(item.transaction_id),
    date: formatDate(item.occurred_at || item.confirmed_at || item.created_at),
    account: item.wallet,
    event: reasonText,
    source: `Journal quarantine · ${formatDirection(item.direction)}`,
    amount: formatMsatAmount(amountMsat, item.asset),
    basis: basisLabel(reason, item.detail),
    impact: "Held from reports",
    status,
    priority,
    owner: profile ?? "Active book",
    evidenceHint: evidenceHint(reason, item.detail),
    nextAction: nextAction(reason),
    metricFilterIds: quarantineReasonFilterIds(reason),
  };
}

function quarantineMetrics(summary: QuarantineSnapshot["summary"]): ReviewMetric[] {
  const missingPrices = countReasons(summary.by_reason, "price");
  const transferReview = countReasons(summary.by_reason, "transfer", "pair", "swap");
  const basisReview = countReasons(summary.by_reason, "basis", "lot", "insufficient");
  const other = Math.max(
    summary.count - missingPrices - transferReview - basisReview,
    0,
  );
  return [
    {
      label: "Quarantined",
      value: summary.count,
      tone: summary.count ? "alert" : "good",
      filterId: "all",
      filterLabel: "All quarantined",
    },
    {
      label: "Missing prices",
      value: missingPrices,
      tone: missingPrices ? "warning" : "neutral",
      filterId: "missing-prices",
    },
    {
      label: "Basis or pairs",
      value: transferReview + basisReview,
      tone: transferReview + basisReview ? "alert" : "neutral",
      filterId: "basis-or-pairs",
    },
    {
      label: "Other review",
      value: other,
      tone: other ? "warning" : "neutral",
      filterId: "other-review",
    },
  ];
}

function quarantineReasonFilterIds(reason: string) {
  const normalized = reason.toLowerCase();
  const filters: string[] = [];
  if (normalized.includes("price")) filters.push("missing-prices");
  if (
    normalized.includes("transfer") ||
    normalized.includes("pair") ||
    normalized.includes("swap") ||
    normalized.includes("basis") ||
    normalized.includes("lot") ||
    normalized.includes("insufficient")
  ) {
    filters.push("basis-or-pairs");
  }
  if (!filters.length) filters.push("other-review");
  return filters;
}

function countReasons(reasons: QuarantineReason[], ...needles: string[]) {
  return reasons.reduce((total, row) => {
    const normalized = row.reason.toLowerCase();
    return needles.some((needle) => normalized.includes(needle))
      ? total + row.count
      : total;
  }, 0);
}

function formatReason(reason: string) {
  return reason
    .replaceAll("_", " ")
    .replace(/\b\w/g, (letter) => letter.toUpperCase());
}

function quarantinePriority(reason: string): ReviewTableRow["priority"] {
  const normalized = reason.toLowerCase();
  if (
    normalized.includes("unsupported") ||
    normalized.includes("insufficient") ||
    normalized.includes("basis") ||
    normalized.includes("asset")
  ) {
    return "High";
  }
  if (normalized.includes("price") || normalized.includes("transfer")) {
    return "Medium";
  }
  return "Medium";
}

function basisLabel(reason: string, detail: Record<string, unknown>) {
  const normalized = reason.toLowerCase();
  if (normalized.includes("price")) return "Missing fiat price";
  if (normalized.includes("basis") || normalized.includes("lot")) {
    return "Missing cost basis";
  }
  if (normalized.includes("transfer") || normalized.includes("pair")) {
    return "Pairing decision";
  }
  const detailReason = typeof detail.reason === "string" ? detail.reason : "";
  return detailReason ? formatReason(detailReason) : "Review required";
}

function evidenceHint(reason: string, detail: Record<string, unknown>) {
  const normalized = reason.toLowerCase();
  if (normalized.includes("price")) return "Add a fiat price or rates coverage";
  if (normalized.includes("transfer") || normalized.includes("pair")) {
    return "Choose or dismiss the matching movement";
  }
  if (normalized.includes("basis") || normalized.includes("lot")) {
    return "Review acquisition history and cost basis";
  }
  if (normalized.includes("asset")) return "Map the source asset before reporting";
  const keys = Object.keys(detail);
  return keys.length ? `Review ${keys.slice(0, 2).join(", ")}` : "Review evidence";
}

function nextAction(reason: string) {
  const normalized = reason.toLowerCase();
  if (normalized.includes("price")) return "Set price, then process journals";
  if (normalized.includes("transfer") || normalized.includes("pair")) {
    return "Review pair, then process journals";
  }
  if (normalized.includes("basis") || normalized.includes("lot")) {
    return "Add missing acquisition context";
  }
  return "Resolve the source issue";
}

function formatDirection(direction: string) {
  if (direction === "inbound") return "inbound";
  if (direction === "outbound") return "outbound";
  return direction || "transaction";
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
