import type {
  ReviewMetric,
  ReviewTableRow,
} from "@/components/kb/ReviewDataTable";

import type { QuarantineItem, QuarantineReason, QuarantineSnapshot } from "./types";

export function quarantineItemToRow(
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
    transactionAction: {
      transactionId: item.transaction_id,
      label: actionLabel(reason),
      tab: actionTab(reason),
    },
  };
}

export function quarantineRows(snapshot: QuarantineSnapshot): ReviewTableRow[] {
  return snapshot.items.map((item) =>
    quarantineItemToRow(item, snapshot.summary.profile),
  );
}

export function quarantineMetrics(
  summary: QuarantineSnapshot["summary"],
): ReviewMetric[] {
  const missingPrices = countReasons(summary.by_reason, "price", "pricing_review");
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

export function quarantineReasonFilterIds(reason: string) {
  // Mirrors daemon quarantine reason strings until the API exposes a typed
  // review category for this filter band.
  const normalized = reason.toLowerCase();
  const filters: string[] = [];
  if (normalized.includes("price") || normalized.includes("pricing_review")) {
    filters.push("missing-prices");
  }
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
  if (normalized.includes("transfer_fee_implausible")) {
    return "Split transfer / swap review";
  }
  if (normalized.includes("pricing_review")) return "Coarse pricing — verify";
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
  if (normalized.includes("transfer_fee_implausible")) {
    return "Review the self-transfer and residual swap or payout leg";
  }
  if (normalized.includes("pricing_review")) {
    return "Priced from daily rates — fetch precise prices or confirm";
  }
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
  if (normalized.includes("transfer_fee_implausible")) {
    return "Review split transfer/swap, then process journals";
  }
  if (normalized.includes("pricing_review")) {
    return "Fetch precise prices or confirm, then process journals";
  }
  if (normalized.includes("price")) return "Set price, then process journals";
  if (normalized.includes("transfer") || normalized.includes("pair")) {
    return "Review pair, then process journals";
  }
  if (normalized.includes("basis") || normalized.includes("lot")) {
    return "Add missing acquisition context";
  }
  return "Resolve the source issue";
}

function actionLabel(reason: string) {
  const normalized = reason.toLowerCase();
  if (normalized.includes("transfer_fee_implausible")) return "Open transaction";
  if (normalized.includes("pricing_review")) return "Open pricing";
  if (normalized.includes("price")) return "Open pricing";
  if (normalized.includes("transfer") || normalized.includes("pair")) {
    return "Open pairing";
  }
  if (normalized.includes("basis") || normalized.includes("lot")) {
    return "Open tax review";
  }
  return "Open transaction";
}

function actionTab(reason: string): NonNullable<ReviewTableRow["transactionAction"]>["tab"] {
  const normalized = reason.toLowerCase();
  if (normalized.includes("pricing_review")) return "pricing";
  if (normalized.includes("price")) return "pricing";
  if (normalized.includes("basis") || normalized.includes("lot")) return "tax";
  return "details";
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
