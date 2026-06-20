import type { TFunction } from "i18next";

import type {
  ReviewMetric,
  ReviewTableRow,
} from "@/components/kb/ReviewDataTable";

import type { QuarantineItem, QuarantineReason, QuarantineSnapshot } from "./types";

export function quarantineItemToRow(
  item: QuarantineItem,
  profile: string | null,
  t: TFunction<"journals">,
): ReviewTableRow {
  const reason = item.reason || "review_required";
  const reasonText = formatReason(reason);
  const priority = quarantinePriority(reason);
  const status = priority === "High" ? "Blocked" : "Needs review";
  const amountMsat =
    item.direction === "outbound" ? -Math.abs(item.amount_msat) : item.amount_msat;
  return {
    id: item.external_id || shortId(item.transaction_id),
    date: formatDate(item.occurred_at || item.confirmed_at || item.created_at, t),
    account: item.wallet,
    event: reasonText,
    source: t("quarantine.source", { direction: formatDirection(item.direction, t) }),
    amount: formatMsatAmount(amountMsat, item.asset),
    basis: basisLabel(reason, item.detail, t),
    impact: t("quarantine.impact"),
    status,
    priority,
    owner: profile ?? t("quarantine.ownerFallback"),
    evidenceHint: evidenceHint(reason, item.detail, t),
    nextAction: nextAction(reason, t),
    metricFilterIds: quarantineReasonFilterIds(reason),
    transactionAction: {
      transactionId: item.transaction_id,
      label: actionLabel(reason, t),
      tab: actionTab(reason),
    },
  };
}

export function quarantineRows(
  snapshot: QuarantineSnapshot,
  t: TFunction<"journals">,
): ReviewTableRow[] {
  return snapshot.items.map((item) =>
    quarantineItemToRow(item, snapshot.summary.profile, t),
  );
}

export function quarantineMetrics(
  summary: QuarantineSnapshot["summary"],
  t: TFunction<"journals">,
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
      label: t("quarantine.metric.quarantined"),
      value: summary.count,
      tone: summary.count ? "alert" : "good",
      filterId: "all",
      filterLabel: t("quarantine.metric.quarantinedFilter"),
    },
    {
      label: t("quarantine.metric.missingPrices"),
      value: missingPrices,
      tone: missingPrices ? "warning" : "neutral",
      filterId: "missing-prices",
    },
    {
      label: t("quarantine.metric.basisOrPairs"),
      value: transferReview + basisReview,
      tone: transferReview + basisReview ? "alert" : "neutral",
      filterId: "basis-or-pairs",
    },
    {
      label: t("quarantine.metric.otherReview"),
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

// Structural humanizer for the daemon reason CODE (e.g. "missing_price" →
// "Missing price"). The reason code is an open, stable id from the daemon, so
// this derives a readable label from the code itself rather than a fixed
// translated phrase. The semantic copy (basis/evidence/next action) is
// translated via the keys below.
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

function basisLabel(
  reason: string,
  detail: Record<string, unknown>,
  t: TFunction<"journals">,
) {
  const normalized = reason.toLowerCase();
  if (normalized.includes("transfer_fee_implausible")) {
    return t("quarantine.basis.splitTransfer");
  }
  if (normalized.includes("ownership_transfer")) {
    return t("quarantine.basis.ownershipTransfer");
  }
  if (normalized.includes("pricing_review")) {
    return t("quarantine.basis.coarsePricing");
  }
  if (normalized.includes("price")) return t("quarantine.basis.missingPrice");
  if (normalized.includes("basis") || normalized.includes("lot")) {
    return t("quarantine.basis.missingBasis");
  }
  if (normalized.includes("transfer") || normalized.includes("pair")) {
    return t("quarantine.basis.pairing");
  }
  const detailReason = typeof detail.reason === "string" ? detail.reason : "";
  return detailReason ? formatReason(detailReason) : t("quarantine.basis.reviewRequired");
}

function evidenceHint(
  reason: string,
  detail: Record<string, unknown>,
  t: TFunction<"journals">,
) {
  const normalized = reason.toLowerCase();
  if (normalized.includes("transfer_fee_implausible")) {
    return t("quarantine.evidence.splitTransfer");
  }
  if (normalized.includes("ownership_transfer")) {
    return t("quarantine.evidence.ownershipTransfer");
  }
  if (normalized.includes("pricing_review")) {
    return t("quarantine.evidence.coarsePricing");
  }
  if (normalized.includes("price")) return t("quarantine.evidence.price");
  if (normalized.includes("transfer") || normalized.includes("pair")) {
    return t("quarantine.evidence.pair");
  }
  if (normalized.includes("basis") || normalized.includes("lot")) {
    return t("quarantine.evidence.basis");
  }
  if (normalized.includes("asset")) return t("quarantine.evidence.asset");
  const keys = Object.keys(detail);
  return keys.length
    ? t("quarantine.evidence.detail", { keys: keys.slice(0, 2).join(", ") })
    : t("quarantine.evidence.fallback");
}

function nextAction(reason: string, t: TFunction<"journals">) {
  const normalized = reason.toLowerCase();
  if (normalized.includes("transfer_fee_implausible")) {
    return t("quarantine.nextAction.splitTransfer");
  }
  if (normalized.includes("ownership_transfer_source_ambiguous")) {
    return t("quarantine.nextAction.ownershipConsolidation");
  }
  if (normalized.includes("ownership_transfer_amount_mismatch")) {
    return t("quarantine.nextAction.ownershipAmountMismatch");
  }
  if (normalized.includes("ownership_transfer")) {
    return t("quarantine.nextAction.ownershipTransfer");
  }
  if (normalized.includes("pricing_review")) {
    return t("quarantine.nextAction.coarsePricing");
  }
  if (normalized.includes("price")) return t("quarantine.nextAction.price");
  if (normalized.includes("transfer") || normalized.includes("pair")) {
    return t("quarantine.nextAction.pair");
  }
  if (normalized.includes("basis") || normalized.includes("lot")) {
    return t("quarantine.nextAction.basis");
  }
  return t("quarantine.nextAction.fallback");
}

function actionLabel(reason: string, t: TFunction<"journals">) {
  const normalized = reason.toLowerCase();
  if (normalized.includes("transfer_fee_implausible")) {
    return t("quarantine.action.openTransaction");
  }
  if (normalized.includes("ownership_transfer")) {
    return t("quarantine.action.openPairing");
  }
  if (normalized.includes("pricing_review")) return t("quarantine.action.openPricing");
  if (normalized.includes("price")) return t("quarantine.action.openPricing");
  if (normalized.includes("transfer") || normalized.includes("pair")) {
    return t("quarantine.action.openPairing");
  }
  if (normalized.includes("basis") || normalized.includes("lot")) {
    return t("quarantine.action.openTaxReview");
  }
  return t("quarantine.action.openTransaction");
}

function actionTab(reason: string): NonNullable<ReviewTableRow["transactionAction"]>["tab"] {
  const normalized = reason.toLowerCase();
  if (normalized.includes("pricing_review")) return "pricing";
  if (normalized.includes("price")) return "pricing";
  if (normalized.includes("basis") || normalized.includes("lot")) return "tax";
  return "details";
}

function formatDirection(direction: string, t: TFunction<"journals">) {
  if (direction === "inbound") return t("quarantine.direction.inbound");
  if (direction === "outbound") return t("quarantine.direction.outbound");
  return direction || t("quarantine.direction.fallback");
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
