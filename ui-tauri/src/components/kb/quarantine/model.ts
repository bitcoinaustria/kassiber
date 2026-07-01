import type { TFunction } from "i18next";

import type {
  ReviewMetric,
  ReviewTableRow,
  ReviewTone,
} from "@/components/kb/ReviewDataTable";

import type { QuarantineItem, QuarantineReason, QuarantineSnapshot } from "./types";

export type QuarantineResolveStepId =
  | "sync-wallets"
  | "review-transfers"
  | "fix-basis"
  | "add-prices"
  | "review-other"
  | "process-journals";

export type QuarantineResolveActionKind =
  | "open-row"
  | "process-journals"
  | "none";

export interface QuarantineResolveStep {
  id: QuarantineResolveStepId;
  count: number;
  tone: ReviewTone;
  title: string;
  detail: string;
  actionLabel: string;
  actionKind: QuarantineResolveActionKind;
  primaryAction?: NonNullable<ReviewTableRow["transactionAction"]>;
  rowIds: string[];
  previewRows: Array<{
    id: string;
    event: string;
    account: string;
    amount: string;
  }>;
}

export interface QuarantineResolvePlan {
  total: number;
  summary: string;
  steps: QuarantineResolveStep[];
  actionableCount: number;
  blockedCount: number;
}

export function quarantineItemToRow(
  item: QuarantineItem,
  profile: string | null,
  t: TFunction<"journals">,
): ReviewTableRow {
  const reason = item.reason || "review_required";
  const priority = quarantinePriority(reason);
  const status = priority === "High" ? "Blocked" : "Needs review";
  const amountMsat =
    item.direction === "outbound" ? -Math.abs(item.amount_msat) : item.amount_msat;
  return {
    id: item.external_id || shortId(item.transaction_id),
    date: formatDate(item.occurred_at || item.confirmed_at || item.created_at, t),
    account: item.wallet,
    event: issueLabel(reason, item, t),
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

export function quarantineResolvePlan(
  snapshot: QuarantineSnapshot,
  rows: ReviewTableRow[],
  t: TFunction<"journals">,
): QuarantineResolvePlan {
  // `rows` is produced by `quarantineRows(snapshot)`, so it is a 1:1,
  // order-preserving projection of `snapshot.items`. Group by index rather
  // than by transaction id: several quarantine items can share one
  // transaction id (e.g. split-transfer legs), and a txid-keyed map would
  // collapse them onto a single row and mis-count / duplicate the steps.
  const groups = new Map<QuarantineResolveStepId, ReviewTableRow[]>();
  snapshot.items.forEach((item, index) => {
    const row = rows[index];
    if (!row) return;
    const groupId = resolveStepIdForReason(item.reason || "review_required");
    const groupRows = groups.get(groupId) ?? [];
    groupRows.push(row);
    groups.set(groupId, groupRows);
  });

  const steps: QuarantineResolveStep[] = [];
  for (const id of RESOLVE_STEP_ORDER) {
    const groupRows = groups.get(id);
    if (!groupRows?.length) continue;
    steps.push(resolveStep(id, groupRows, t));
  }
  if (snapshot.summary.count > 0) {
    steps.push({
      id: "process-journals",
      count: snapshot.summary.count,
      tone: "good",
      title: t("quarantine.resolvePlan.step.process.title"),
      detail: t("quarantine.resolvePlan.step.process.detail"),
      actionLabel: t("quarantine.resolvePlan.step.process.action"),
      actionKind: "process-journals",
      rowIds: rows.map((row) => row.id),
      previewRows: [],
    });
  }

  const actionableCount = steps.filter((step) => step.actionKind === "open-row").length;
  const blockedCount = rows.filter((row) => row.status === "Blocked").length;
  return {
    total: snapshot.summary.count,
    summary:
      snapshot.summary.count > 0
        ? t("quarantine.resolvePlan.summary", {
            count: snapshot.summary.count,
            steps: Math.max(steps.length - 1, 0),
          })
        : t("quarantine.resolvePlan.summaryClear"),
    steps,
    actionableCount,
    blockedCount,
  };
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

const RESOLVE_STEP_ORDER: QuarantineResolveStepId[] = [
  "sync-wallets",
  "review-transfers",
  "fix-basis",
  "add-prices",
  "review-other",
];

function resolveStepIdForReason(reason: string): QuarantineResolveStepId {
  const normalized = reason.toLowerCase();
  if (normalized.includes("ownership_transfer_amount_mismatch")) {
    return "sync-wallets";
  }
  if (
    normalized.includes("transfer_fee_implausible") ||
    normalized.includes("ownership_transfer") ||
    normalized.includes("derived_transfer") ||
    normalized.includes("transfer") ||
    normalized.includes("pair") ||
    normalized.includes("swap")
  ) {
    return "review-transfers";
  }
  if (
    normalized.includes("basis") ||
    normalized.includes("lot") ||
    normalized.includes("insufficient")
  ) {
    return "fix-basis";
  }
  if (normalized.includes("price") || normalized.includes("pricing_review")) {
    return "add-prices";
  }
  return "review-other";
}

function resolveStep(
  id: QuarantineResolveStepId,
  rows: ReviewTableRow[],
  t: TFunction<"journals">,
): QuarantineResolveStep {
  const primaryAction = rows.find((row) => row.transactionAction)?.transactionAction;
  return {
    id,
    count: rows.length,
    tone: resolveStepTone(id, rows),
    title: resolveStepTitle(id, rows.length, t),
    detail: resolveStepDetail(id, rows.length, t),
    actionLabel: resolveStepActionLabel(id, t),
    actionKind: primaryAction ? "open-row" : "none",
    primaryAction,
    rowIds: rows.map((row) => row.id),
    previewRows: rows.slice(0, 3).map((row) => ({
      id: row.id,
      event: row.event,
      account: row.account,
      amount: row.amount,
    })),
  };
}

function resolveStepTone(
  id: QuarantineResolveStepId,
  rows: ReviewTableRow[],
): ReviewTone {
  if (id === "sync-wallets" || rows.some((row) => row.status === "Blocked")) {
    return "alert";
  }
  if (id === "add-prices" || id === "review-transfers") return "warning";
  return "neutral";
}

function resolveStepTitle(
  id: QuarantineResolveStepId,
  count: number,
  t: TFunction<"journals">,
) {
  switch (id) {
    case "sync-wallets":
      return t("quarantine.resolvePlan.step.sync-wallets.title", { count });
    case "review-transfers":
      return t("quarantine.resolvePlan.step.review-transfers.title", { count });
    case "fix-basis":
      return t("quarantine.resolvePlan.step.fix-basis.title", { count });
    case "add-prices":
      return t("quarantine.resolvePlan.step.add-prices.title", { count });
    case "review-other":
      return t("quarantine.resolvePlan.step.review-other.title", { count });
    case "process-journals":
      return t("quarantine.resolvePlan.step.process.title", { count });
  }
}

function resolveStepDetail(
  id: QuarantineResolveStepId,
  count: number,
  t: TFunction<"journals">,
) {
  switch (id) {
    case "sync-wallets":
      return t("quarantine.resolvePlan.step.sync-wallets.detail", { count });
    case "review-transfers":
      return t("quarantine.resolvePlan.step.review-transfers.detail", { count });
    case "fix-basis":
      return t("quarantine.resolvePlan.step.fix-basis.detail", { count });
    case "add-prices":
      return t("quarantine.resolvePlan.step.add-prices.detail", { count });
    case "review-other":
      return t("quarantine.resolvePlan.step.review-other.detail", { count });
    case "process-journals":
      return t("quarantine.resolvePlan.step.process.detail", { count });
  }
}

function resolveStepActionLabel(
  id: QuarantineResolveStepId,
  t: TFunction<"journals">,
) {
  switch (id) {
    case "sync-wallets":
      return t("quarantine.resolvePlan.step.sync-wallets.action");
    case "review-transfers":
      return t("quarantine.resolvePlan.step.review-transfers.action");
    case "fix-basis":
      return t("quarantine.resolvePlan.step.fix-basis.action");
    case "add-prices":
      return t("quarantine.resolvePlan.step.add-prices.action");
    case "review-other":
      return t("quarantine.resolvePlan.step.review-other.action");
    case "process-journals":
      return t("quarantine.resolvePlan.step.process.action");
  }
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

function issueLabel(
  reason: string,
  item: QuarantineItem,
  t: TFunction<"journals">,
) {
  const normalized = reason.toLowerCase();
  const detail = item.detail;
  const asset = detailString(detail, "asset") || item.asset;
  const fromWallet = detailString(detail, "from_wallet");
  const toWallet = detailString(detail, "to_wallet");
  const wallet = detailString(detail, "wallet") || item.wallet;
  const direction =
    detailString(detail, "direction") || formatDirection(item.direction, t);
  if (normalized.includes("transfer_fee_implausible")) {
    return fromWallet && toWallet
      ? t("quarantine.issue.implausibleFeeWithWallets", {
          from: fromWallet,
          to: toWallet,
        })
      : t("quarantine.issue.implausibleFee");
  }
  if (normalized.includes("pricing_review")) {
    return t("quarantine.issue.pricingReview", {
      asset,
      granularity:
        detailString(detail, "pricing_granularity") ||
        t("quarantine.detailFallback.price"),
    });
  }
  if (normalized.includes("price")) {
    return t("quarantine.issue.missingPrice", { asset, direction });
  }
  if (normalized.includes("ownership_transfer")) {
    return t("quarantine.issue.ownershipTransfer", { wallet });
  }
  if (normalized.includes("derived_transfer_group_blocked")) {
    return t("quarantine.issue.transferGroupBlocked", {
      reason: formatReason(detailString(detail, "blocked_by_reason") || reason),
    });
  }
  if (normalized.includes("insufficient_lots")) {
    return t("quarantine.issue.insufficientLots", { asset, wallet });
  }
  if (normalized.includes("missing_cost_basis") || normalized.includes("basis")) {
    return t("quarantine.issue.missingBasis", { asset, wallet });
  }
  return formatReason(reason);
}

function evidenceHint(
  reason: string,
  detail: Record<string, unknown>,
  t: TFunction<"journals">,
) {
  const normalized = reason.toLowerCase();
  if (normalized.includes("transfer_fee_implausible")) {
    const impliedFee = formatBtcDetail(detailNumber(detail, "implied_fee"));
    const ceiling = formatBtcDetail(detailNumber(detail, "fee_ceiling"));
    if (impliedFee && ceiling) {
      return t("quarantine.evidence.splitTransferValues", {
        impliedFee,
        ceiling,
      });
    }
    return t("quarantine.evidence.splitTransfer");
  }
  if (normalized.includes("ownership_transfer")) {
    const rowAmount = formatMsatDetail(detailNumber(detail, "row_amount_msat"));
    const ownedOutputs = formatMsatDetail(
      detailNumber(detail, "owned_outputs_msat"),
    );
    const legAmount = formatMsatDetail(detailNumber(detail, "leg_amount_msat"));
    if (rowAmount && ownedOutputs) {
      return t("quarantine.evidence.ownershipAmountMismatch", {
        rowAmount,
        ownedOutputs,
      });
    }
    if (legAmount) {
      return t("quarantine.evidence.ownershipLeg", { amount: legAmount });
    }
    return t("quarantine.evidence.ownershipTransfer");
  }
  if (normalized.includes("pricing_review")) {
    const quality = detailString(detail, "pricing_quality");
    const granularity = detailString(detail, "pricing_granularity");
    const wallet = detailString(detail, "wallet");
    if (quality || granularity || wallet) {
      return t("quarantine.evidence.coarsePricingValues", {
        quality: quality || t("quarantine.detailFallback.price"),
        granularity: granularity || t("quarantine.detailFallback.price"),
        wallet: wallet || t("quarantine.detailFallback.wallet"),
      });
    }
    return t("quarantine.evidence.coarsePricing");
  }
  if (normalized.includes("price")) {
    const requiredFor = detailString(detail, "required_for");
    const fromWallet = detailString(detail, "from_wallet");
    const toWallet = detailString(detail, "to_wallet");
    if (requiredFor || fromWallet || toWallet) {
      return t("quarantine.evidence.priceValues", {
        requiredFor: requiredFor
          ? formatReason(requiredFor)
          : t("quarantine.detailFallback.price"),
        route:
          fromWallet && toWallet
            ? `${fromWallet} -> ${toWallet}`
            : t("quarantine.detailFallback.wallet"),
      });
    }
    return t("quarantine.evidence.price");
  }
  if (normalized.includes("transfer") || normalized.includes("pair")) {
    return t("quarantine.evidence.pair");
  }
  if (normalized.includes("basis") || normalized.includes("lot")) {
    const required = formatBtcDetail(detailNumber(detail, "required"));
    const available =
      formatBtcDetail(detailNumber(detail, "available")) ||
      formatBtcDetail(detailNumber(detail, "priced_available"));
    const fromWallet = detailString(detail, "from_wallet");
    if (required && available) {
      return t("quarantine.evidence.basisValues", {
        required,
        available,
        wallet: fromWallet || t("quarantine.detailFallback.wallet"),
      });
    }
    return t("quarantine.evidence.basis");
  }
  if (normalized.includes("asset")) return t("quarantine.evidence.asset");
  const keys = Object.keys(detail);
  return keys.length
    ? t("quarantine.evidence.detail", { keys: keys.slice(0, 2).join(", ") })
    : t("quarantine.evidence.fallback");
}

function detailString(detail: Record<string, unknown>, key: string) {
  const value = detail[key];
  return typeof value === "string" && value.trim() ? value.trim() : "";
}

function detailNumber(detail: Record<string, unknown>, key: string) {
  const value = detail[key];
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    return Number.isFinite(parsed) ? parsed : null;
  }
  return null;
}

function formatBtcDetail(value: number | null) {
  if (value === null) return "";
  const sats = Math.round(Math.abs(value) * 100_000_000);
  return `${sats.toLocaleString("en-US")} sats`;
}

function formatMsatDetail(value: number | null) {
  if (value === null) return "";
  const sats = Math.round(Math.abs(value) / 1000);
  return `${sats.toLocaleString("en-US")} sats`;
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
