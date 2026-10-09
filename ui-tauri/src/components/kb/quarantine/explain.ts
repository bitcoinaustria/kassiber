import type { TFunction } from "i18next";

import { formatSats } from "@/lib/localeFormat";

import type {
  QuarantineAction,
  QuarantineCategory,
  QuarantineEvidence,
  QuarantineGroup,
  QuarantineItem,
} from "./types";

// One reading of a quarantine reason for every surface (cause cards, the
// review table, the transaction sheet). The daemon owns the classification
// (category, root, blocking state, actions); this module only turns the
// stable reason code plus its evidence into words. Unknown reasons fall back
// to their category so no raw code ever reaches the user.

export type CauseKey =
  | "ownershipSourceMissing"
  | "ownershipSourceAmbiguous"
  | "ownershipDestinationAmbiguous"
  | "ownershipDestinationMissingRef"
  | "ownershipAmountMismatch"
  | "ownershipConservation"
  | "ownershipFeeEvidence"
  | "ownershipAssetEvidence"
  | "ownershipAmbiguousOutput"
  | "ownershipDuplicateOutbound"
  | "liquidGraphIncomplete"
  | "recordedAuthorityConflict"
  | "chronologyMismatch"
  | "networkMismatch"
  | "feeImplausible"
  | "privacyHop"
  | "samouraiUnverified"
  | "channelOpen"
  | "channelClose"
  | "nativeTransitionAmbiguous"
  | "nativeTransitionAmountMismatch"
  | "nativeTransitionFeeTiming"
  | "migrationIncomplete"
  | "gapHold"
  | "implicitDelta"
  | "unclaimedResidual"
  | "sourceOverlap"
  | "searchCapacity"
  | "componentProblem"
  | "reviewedSuspense"
  | "reviewedSuspensePair"
  | "quantityOther"
  | "basisBarrier"
  | "pairDependency"
  | "provenanceIncomplete"
  | "groupBlocked"
  | "carryUnresolved"
  | "missingPrice"
  | "pricingReview"
  | "swapPriceRequired"
  | "insufficientLots"
  | "missingCostBasis"
  | "unclassifiedIncome"
  | "nonSaleDisposal"
  | "valuationUnsupported"
  | "conflictingSpend"
  | "pendingConfirmation"
  | "unscopedTransfer"
  | "ownedFanout"
  | "pairAmbiguous";

const REASON_CAUSES: Record<string, CauseKey> = {
  ownership_transfer_source_missing: "ownershipSourceMissing",
  ownership_transfer_source_ambiguous: "ownershipSourceAmbiguous",
  ownership_transfer_destination_ambiguous: "ownershipDestinationAmbiguous",
  ownership_transfer_destination_missing_ref: "ownershipDestinationMissingRef",
  ownership_transfer_amount_mismatch: "ownershipAmountMismatch",
  ownership_transfer_conservation_mismatch: "ownershipConservation",
  ownership_transfer_fee_evidence_incomplete: "ownershipFeeEvidence",
  ownership_transfer_asset_evidence_incomplete: "ownershipAssetEvidence",
  ownership_transfer_ambiguous_output: "ownershipAmbiguousOutput",
  ownership_transfer_duplicate_outbound: "ownershipDuplicateOutbound",
  liquid_transfer_graph_incomplete: "liquidGraphIncomplete",
  recorded_transfer_authority_conflict: "recordedAuthorityConflict",
  transfer_pair_chronology_mismatch: "chronologyMismatch",
  transfer_network_mismatch: "networkMismatch",
  transfer_fee_implausible: "feeImplausible",
  privacy_hop_unresolved: "privacyHop",
  samourai_native_event_unverified: "samouraiUnverified",
  channel_open_unresolved: "channelOpen",
  channel_close_unresolved: "channelClose",
  native_transition_ambiguous: "nativeTransitionAmbiguous",
  native_transition_amount_mismatch: "nativeTransitionAmountMismatch",
  native_transition_fee_timing_unresolved: "nativeTransitionFeeTiming",
  custody_authored_migration_incomplete: "migrationIncomplete",
  custody_basis_barrier: "basisBarrier",
  transfer_pair_dependency_blocked: "pairDependency",
  basis_provenance_incomplete: "provenanceIncomplete",
  derived_transfer_group_blocked: "groupBlocked",
  at_swap_basis_carry_unresolved: "carryUnresolved",
  bitcoin_rail_carry_basis_unresolved: "carryUnresolved",
  missing_spot_price: "missingPrice",
  pricing_review_required: "pricingReview",
  at_swap_price_required: "swapPriceRequired",
  insufficient_lots: "insufficientLots",
  missing_cost_basis: "missingCostBasis",
  unclassified_income_kind: "unclassifiedIncome",
  non_sale_disposal_kind: "nonSaleDisposal",
  acquisition_valuation_unsupported: "valuationUnsupported",
  conflicting_spend: "conflictingSpend",
  pending_onchain_confirmation: "pendingConfirmation",
  unscoped_transfer_review: "unscopedTransfer",
  owned_fanout_unresolved: "ownedFanout",
  manual_multi_pair_ambiguous: "pairAmbiguous",
  transfer_mismatch: "pairAmbiguous",
  custody_component_blocked: "componentProblem",
  custody_interpreter_blocked: "quantityOther",
};

const BLOCKER_CAUSES: Record<string, CauseKey> = {
  custody_gap_review_required: "gapHold",
  implicit_wallet_delta_unallocated: "implicitDelta",
  unclaimed_source_residual: "unclaimedResidual",
  source_overlap_quantity_unresolved: "sourceOverlap",
  search_capacity_incomplete: "searchCapacity",
  capacity_source_suspense_required: "searchCapacity",
  reviewed_residual_suspense: "reviewedSuspense",
};

/** Every reason the daemon can store; the copy test keeps both locales complete. */
export const KNOWN_QUARANTINE_REASONS = Object.keys(REASON_CAUSES);
export const KNOWN_QUANTITY_BLOCKERS = Object.keys(BLOCKER_CAUSES);
export const CAUSE_KEYS = Array.from(
  new Set<CauseKey>([
    ...Object.values(REASON_CAUSES),
    ...Object.values(BLOCKER_CAUSES),
    "componentProblem",
    "quantityOther",
    "reviewedSuspensePair",
  ]),
);

// Older daemons do not send a category; derive the same one from the reason
// so the page still groups and prioritizes consistently.
const CAUSE_CATEGORIES: Record<CauseKey, QuarantineCategory> = {
  ownershipSourceMissing: "missing_wallet_history",
  ownershipDestinationMissingRef: "missing_wallet_history",
  channelOpen: "missing_wallet_history",
  channelClose: "missing_wallet_history",
  gapHold: "missing_wallet_history",
  implicitDelta: "missing_wallet_history",
  ownershipAmountMismatch: "missing_chain_evidence",
  ownershipConservation: "missing_chain_evidence",
  ownershipFeeEvidence: "missing_chain_evidence",
  ownershipAssetEvidence: "missing_chain_evidence",
  liquidGraphIncomplete: "missing_chain_evidence",
  recordedAuthorityConflict: "missing_chain_evidence",
  samouraiUnverified: "missing_chain_evidence",
  conflictingSpend: "missing_chain_evidence",
  pendingConfirmation: "missing_chain_evidence",
  missingPrice: "missing_price",
  pricingReview: "missing_price",
  swapPriceRequired: "missing_price",
  insufficientLots: "missing_acquisition_history",
  missingCostBasis: "missing_acquisition_history",
  nativeTransitionFeeTiming: "unsupported",
  valuationUnsupported: "unsupported",
  basisBarrier: "downstream",
  pairDependency: "downstream",
  provenanceIncomplete: "downstream",
  groupBlocked: "downstream",
  carryUnresolved: "downstream",
  ownershipSourceAmbiguous: "needs_decision",
  ownershipDestinationAmbiguous: "needs_decision",
  ownershipAmbiguousOutput: "needs_decision",
  ownershipDuplicateOutbound: "needs_decision",
  chronologyMismatch: "needs_decision",
  networkMismatch: "needs_decision",
  feeImplausible: "needs_decision",
  privacyHop: "needs_decision",
  nativeTransitionAmbiguous: "needs_decision",
  nativeTransitionAmountMismatch: "needs_decision",
  migrationIncomplete: "needs_decision",
  unclaimedResidual: "needs_decision",
  sourceOverlap: "needs_decision",
  searchCapacity: "needs_decision",
  componentProblem: "needs_decision",
  reviewedSuspense: "needs_decision",
  reviewedSuspensePair: "needs_decision",
  quantityOther: "needs_decision",
  unclassifiedIncome: "needs_decision",
  nonSaleDisposal: "needs_decision",
  unscopedTransfer: "needs_decision",
  ownedFanout: "needs_decision",
  pairAmbiguous: "needs_decision",
};

export function causeKeyFor(
  reason: string,
  evidence?: QuarantineEvidence | null,
  detail?: Record<string, unknown> | null,
): CauseKey | null {
  if (reason === "custody_quantity_unresolved") {
    const blocker =
      evidence?.blocker_code ??
      (typeof detail?.blocker_code === "string" ? detail.blocker_code : "");
    // A suspense left by a pair reads as the pair, which is what to check.
    if (blocker === "reviewed_residual_suspense" && evidence?.pair_id) {
      return "reviewedSuspensePair";
    }
    if (blocker && BLOCKER_CAUSES[blocker]) return BLOCKER_CAUSES[blocker];
    if (blocker.startsWith("custody_component_")) return "componentProblem";
    return "quantityOther";
  }
  return REASON_CAUSES[reason] ?? null;
}

export function categoryForReason(
  reason: string,
  evidence?: QuarantineEvidence | null,
  detail?: Record<string, unknown> | null,
): QuarantineCategory {
  const key = causeKeyFor(reason, evidence, detail);
  return key ? CAUSE_CATEGORIES[key] : "needs_decision";
}

export interface CauseCopy {
  title: string;
  why: string;
  provide: string;
  /** `why` may quote wallet amounts, so it follows "hide sensitive". */
  whyQuotesAmounts: boolean;
}

export interface CauseContext {
  reason: string;
  category?: QuarantineCategory | null;
  evidence?: QuarantineEvidence | null;
  detail?: Record<string, unknown> | null;
  wallet?: string | null;
  asset?: string | null;
  rootLabel?: string | null;
}

// Cause keys are composed from the stable reason code; the copy test proves
// every composed key exists in both locales, which the typed `t` cannot see.
type DynamicT = (key: string, options?: Record<string, unknown>) => string;

function formatMsat(value: number | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value)) return "";
  return formatSats(Math.round(Math.abs(value) / 1000));
}

function joinLabels(labels: string[]) {
  return labels.filter(Boolean).join(", ");
}

/** Title, reason and the information the user has to provide. */
export function causeCopy(context: CauseContext, typedT: TFunction<"journals">): CauseCopy {
  const t = typedT as unknown as DynamicT;
  const evidence = context.evidence ?? {};
  const key = causeKeyFor(context.reason, evidence, context.detail);
  const wallet =
    evidence.wallet_label || context.wallet || t("quarantine.cause.fallback.wallet");
  const wallets =
    joinLabels((evidence.missing_source_wallets ?? []).map((item) => item.label)) ||
    t("quarantine.cause.fallback.sendingWallet");
  const values = {
    wallet,
    wallets,
    asset: context.asset || "BTC",
    required: formatMsat(evidence.required_msat) || t("quarantine.cause.fallback.amount"),
    available: formatMsat(evidence.available_msat) || t("quarantine.cause.fallback.amount"),
    since: evidence.lot_state_uncertain_since?.slice(0, 10) || t("quarantine.cause.fallback.date"),
    root: context.rootLabel || t("quarantine.cause.fallback.root"),
  };
  const whyQuotesAmounts =
    evidence.required_msat != null || evidence.available_msat != null;
  if (key) {
    return {
      title: t(`quarantine.cause.${key}.title`, values),
      why: t(`quarantine.cause.${key}.why`, values),
      provide: t(`quarantine.cause.${key}.provide`, values),
      whyQuotesAmounts,
    };
  }
  const category = context.category ?? "needs_decision";
  return {
    title: t(`quarantine.cause.category.${category}.title`, values),
    why: t(`quarantine.cause.category.${category}.why`, values),
    provide: t(`quarantine.cause.category.${category}.provide`, values),
    whyQuotesAmounts,
  };
}

/**
 * Observed facts behind a cause, in the user's words: what Kassiber saw that
 * the user can check, not a verdict.
 */
export function causeFacts(
  evidence: QuarantineEvidence | null | undefined,
  t: TFunction<"journals">,
): string[] {
  const facts: string[] = [];
  if (evidence?.pair_txids_differ) facts.push(t("quarantine.fact.pairTxidsDiffer"));
  if (evidence?.pair_receipt_before_spend) facts.push(t("quarantine.fact.pairReceiptFirst"));
  return facts;
}

export function categoryLabel(category: QuarantineCategory, t: TFunction<"journals">) {
  return (t as unknown as DynamicT)(`quarantine.category.${category}`);
}

export type QuarantineSheetTab = "details" | "classify" | "pricing" | "tax" | "linked" | "ledger";

/** Which transaction-sheet tab holds the evidence for this cause. */
export function sheetTabForCause(
  reason: string,
  category: QuarantineCategory,
  evidence?: QuarantineEvidence | null,
): QuarantineSheetTab {
  const key = causeKeyFor(reason, evidence);
  if (category === "missing_price") return "pricing";
  if (key === "unclassifiedIncome" || key === "nonSaleDisposal" || key === "valuationUnsupported") {
    return "tax";
  }
  if (category === "missing_acquisition_history") return "tax";
  if (
    key === "reviewedSuspensePair" ||
    key === "ownershipSourceAmbiguous" ||
    key === "ownershipDestinationAmbiguous" ||
    key === "nativeTransitionAmbiguous" ||
    key === "chronologyMismatch" ||
    key === "networkMismatch" ||
    key === "unscopedTransfer" ||
    key === "ownedFanout" ||
    key === "pairAmbiguous"
  ) {
    return "linked";
  }
  return "details";
}

/**
 * Excluding a transaction is a legitimate answer for prices and explicit
 * classification questions. For custody, missing history and follow-on rows
 * it would hide an owned movement or missing basis, never explain it.
 */
// Decision questions about explicit classification or a duplicate import;
// custody decisions (pairing, CoinJoin, components) are never answered by
// excluding a row.
const EXCLUSION_DECISIONS = new Set<CauseKey>([
  "unclassifiedIncome",
  "nonSaleDisposal",
  "ownershipDuplicateOutbound",
]);

/** Exclusion is offered for prices and a few explicit classification questions. */
export function exclusionFitsReason(
  reason: string,
  category: QuarantineCategory,
  evidence?: QuarantineEvidence | null,
) {
  if (category === "missing_price") return true;
  if (category !== "needs_decision") return false;
  const key = causeKeyFor(reason, evidence);
  return key !== null && EXCLUSION_DECISIONS.has(key);
}

export function actionLabel(action: QuarantineAction, t: TFunction<"journals">) {
  switch (action.kind) {
    case "sync_wallet":
      return t("quarantine.cta.syncWallet", {
        wallet: action.wallet_label || t("quarantine.cause.fallback.wallet"),
      });
    case "connect_wallet":
      return t("quarantine.cta.connectWallet");
    case "import_history":
      return t("quarantine.cta.importHistory");
    case "set_price":
      return t("quarantine.cta.setPrice");
    case "classify":
      return t("quarantine.cta.classify");
    case "pair_transfer":
      return t("quarantine.cta.pairTransfer");
    case "review_custody_gap":
      return t("quarantine.cta.reviewCustodyGap");
    case "attach_evidence":
      return t("quarantine.cta.attachEvidence");
    case "wait_for_confirmation":
      return t("quarantine.cta.waitForConfirmation");
    case "resolve_root":
      return t("quarantine.cta.resolveRoot");
    case "review_pair":
      return t("quarantine.cta.reviewPair");
    case "process_journals":
      return t("quarantine.cta.processJournals");
    default:
      return t("quarantine.cta.openTransaction");
  }
}

/** The daemon's reading of one quarantined row, as handed to detail views. */
export interface QuarantineDetailContext {
  reason: string;
  category: QuarantineCategory;
  evidence: QuarantineEvidence;
  rootLabel: string | null;
}

export function quarantineRootLabel(root: NonNullable<QuarantineItem["root"]>) {
  const date = root.occurred_at ? root.occurred_at.slice(0, 10) : "";
  return [date, root.wallet].filter(Boolean).join(", ");
}

/**
 * The reading for the transaction the sheet shows: its row on this page, or
 * the context handed over by the click that opened it from another page.
 */
export function detailContextFor(
  transactionId: string | null,
  items: QuarantineItem[],
  opened: { transactionId: string; context: QuarantineDetailContext } | null,
): QuarantineDetailContext | null {
  if (!transactionId) return null;
  return (
    quarantineDetailContext(items.find((item) => item.transaction_id === transactionId)) ??
    (opened?.transactionId === transactionId ? opened.context : null)
  );
}

/** Context for a cause's root, which may sit on another page of the queue. */
export function quarantineGroupContext(
  group: QuarantineGroup,
): QuarantineDetailContext | null {
  if (!group.root_transaction_id) return null;
  return {
    reason: group.reason,
    category: group.category,
    evidence: group.evidence ?? {},
    rootLabel: null,
  };
}

/** Context for a classified item; older daemons without a category give none. */
export function quarantineDetailContext(
  item: QuarantineItem | null | undefined,
): QuarantineDetailContext | null {
  if (!item?.category) return null;
  return {
    reason: item.reason,
    category: item.category,
    evidence: item.evidence ?? {},
    rootLabel: item.root ? quarantineRootLabel(item.root) : null,
  };
}

/** A held row as one line: when, in which wallet, which transaction. */
export function quarantineRowMeta(item: QuarantineItem) {
  const id = item.external_id;
  const shortId = id.length > 16 ? `${id.slice(0, 8)}…${id.slice(-6)}` : id;
  return [item.occurred_at ? item.occurred_at.slice(0, 10) : "", item.wallet, shortId]
    .filter(Boolean)
    .join(" · ");
}

/** The row's amount with its direction: sats for bitcoin, units otherwise. */
export function quarantineRowAmount(item: QuarantineItem) {
  const sign = item.direction === "outbound" ? "−" : "+";
  const asset = item.asset.toUpperCase();
  if (asset === "BTC" || asset === "LBTC") {
    return `${sign}${formatSats(Math.round(Math.abs(item.amount_msat) / 1000))}`;
  }
  return `${sign}${Math.abs(item.amount)} ${item.asset}`;
}

/** A row waits when it only follows a named cause and clears with it. */
export function isWaiting(item: QuarantineItem) {
  return Boolean(item.is_downstream && item.root);
}

/**
 * Where "Save & next" goes once the list it walks has been re-read: the row
 * after the saved one in the refreshed list, or, when the save (or a sync)
 * cleared the saved row, the first later row of the old list that is still
 * held. A row that cleared meanwhile is skipped, never opened from memory.
 */
export function nextAfterRefresh(
  current: string,
  previous: string[],
  refreshed: string[],
): string | null {
  const at = refreshed.indexOf(current);
  if (at >= 0) return refreshed[at + 1] ?? null;
  const held = new Set(refreshed);
  const index = previous.indexOf(current);
  if (index < 0) return null;
  return previous.slice(index + 1).find((id) => held.has(id)) ?? null;
}

/**
 * Whether a freshly read row is still the pair case the owner confirmed:
 * the same hold, the same pair review and the same two legs. Anything else
 * (cleared, revised, re-pointed) needs a new look before it is unpaired.
 */
export function samePairCase(
  listed: QuarantineItem,
  current: QuarantineItem | null | undefined,
): boolean {
  if (!current) return false;
  const before = listed.evidence ?? {};
  const after = current.evidence ?? {};
  if (!before.pair_id || before.pair_id !== after.pair_id) return false;
  if (listed.reason !== current.reason || before.blocker_code !== after.blocker_code) return false;
  const reading = (evidence: QuarantineEvidence) =>
    JSON.stringify([
      evidence.pair_review ?? null,
      ...(["out", "in"] as const).map((side) => {
        const leg = evidence.pair_legs?.[side];
        return leg
          ? [leg.transaction_id, leg.wallet, leg.asset, leg.amount_msat, leg.occurred_at, leg.external_id]
          : null;
      }),
    ]);
  return reading(before) === reading(after);
}

/** Which sheet tab opens a row, and the reading it is opened with. */
export function quarantineRowTarget(item: QuarantineItem): {
  tab: QuarantineSheetTab;
  context: QuarantineDetailContext | null;
} {
  const category = item.category ?? categoryForReason(item.reason, item.evidence, item.detail);
  return {
    tab: sheetTabForCause(item.reason, category, item.evidence),
    context: quarantineDetailContext(item),
  };
}

/** One review proposal covers at most this many repairs (the daemon's bound). */
export const MAX_FIX_OPERATIONS = 50;

/**
 * The audit reason stored with each unpair. It records the owner's choice;
 * it never asserts the two transactions are unrelated, which Kassiber cannot
 * tell from different txids alone.
 */
export const UNPAIR_REASON = "The owner reviewed this pair from quarantine and chose to unpair it.";

/** The review operations that unpair the pairs the owner picked, once each. */
export function fixOperations(items: QuarantineItem[]) {
  const seen = new Set<string>();
  const operations: Array<{ type: "unpair"; pair_id: string; reason: string }> = [];
  for (const item of items) {
    const pairId = item.evidence?.pair_id;
    if (!pairId || seen.has(pairId)) continue;
    seen.add(pairId);
    operations.push({ type: "unpair", pair_id: pairId, reason: UNPAIR_REASON });
  }
  return operations;
}

/**
 * The owner's picks read against a fresh page: those still held exactly as
 * picked, those that cleared meanwhile, and those that changed and need a
 * new look before anything is unpaired.
 */
export function reconcilePicks(
  picked: QuarantineItem[],
  fresh: QuarantineItem[],
): { current: QuarantineItem[]; cleared: QuarantineItem[]; changed: QuarantineItem[] } {
  const byId = new Map(fresh.map((item) => [item.transaction_id, item]));
  const result = { current: [] as QuarantineItem[], cleared: [] as QuarantineItem[], changed: [] as QuarantineItem[] };
  for (const item of picked) {
    const now = byId.get(item.transaction_id);
    if (!now) result.cleared.push(item);
    else if (samePairCase(item, now)) result.current.push(now);
    else result.changed.push(item);
  }
  return result;
}
