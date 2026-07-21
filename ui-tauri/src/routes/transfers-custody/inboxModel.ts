/**
 * Pure model for the Custody Inbox — the unified decision queue over
 * missing-wallet gap candidates (``ui.custody.gaps.list``) and transfer/swap
 * pairing candidates (``ui.transfers.suggest``).
 *
 * Both feeds ask the user the same underlying question ("do these movements
 * belong together?"), so the inbox merges them into one ranked list of
 * {@link InboxItem}s. Everything here is presentation-side derivation; no
 * daemon contract changes.
 */

import {
  custodyGapActionMode,
  shouldOfferResidualClassification,
  type CustodyGap,
} from "../custodyGapsModel";

/** Structural subset of a pairing candidate the inbox needs. The full shape
 * lives in SwapMatching.tsx; structural typing lets either flow through. */
export interface InboxCandidate {
  out_id: string;
  in_id: string;
  out_asset: string;
  in_asset: string;
  out_amount_msat: number;
  in_amount_msat: number;
  out_wallet_label?: string | null;
  in_wallet_label?: string | null;
  out_wallet_kind: string;
  in_wallet_kind: string;
  out_occurred_at: string;
  in_occurred_at: string;
  confidence: "exact" | "strong";
  method:
    | "payment_hash"
    | "provider_swap_id"
    | "heuristic"
    | "htlc_refund"
    | "ownership_graph";
  swap_fee_msat: number;
  default_kind: string;
  default_policy: string;
  candidate_type?: "transfer" | "swap";
  conflict_set_id: string;
  conflict_size: number;
}

export type InboxItem =
  | { kind: "gap"; id: string; gap: CustodyGap }
  /** A resolved bridge that left an unclassified remainder — the follow-up
   * question ("where did the missing amount go?"). */
  | { kind: "residual"; id: string; gap: CustodyGap }
  | { kind: "candidate"; id: string; candidate: InboxCandidate };

export type InboxFilter = "all" | "blocking" | "suggested";

export function candidateItemId(candidate: InboxCandidate): string {
  return `candidate:${candidate.out_id}->${candidate.in_id}`;
}

/** Same fallback the pairing queue uses: a daemon payload may omit the
 * wallet label, in which case the wallet kind still names the leg. */
export function walletDisplayName(
  label: string | null | undefined,
  kind: string,
): string {
  const trimmed = label?.trim();
  return trimmed || kind || "—";
}

/** A gap whose resolution re-opens later disposals/tax years gates correct
 * reports — those questions surface first. */
export function itemBlocksReports(item: InboxItem): boolean {
  if (item.kind === "candidate") return false;
  return (
    item.gap.downstream.affected_disposals > 0 ||
    item.gap.downstream.affected_years.length > 0
  );
}

/** "Suggested" = the engine itself considers this safe to propose: a
 * promotion-eligible gap, or an exact-evidence candidate with no competing
 * sibling (ownership-graph proofs stay deliberate one-at-a-time reviews but
 * are still exact evidence, so they keep the suggested marker). */
export function itemIsSuggested(item: InboxItem): boolean {
  if (item.kind === "candidate") {
    return (
      item.candidate.confidence === "exact" && item.candidate.conflict_size <= 1
    );
  }
  return item.gap.promotion_eligible;
}

export function itemHasCompetingEvidence(item: InboxItem): boolean {
  if (item.kind === "candidate") return item.candidate.conflict_size > 1;
  return item.gap.status === "conflicting";
}

/** Weak advisory search hints collapse into a quiet group at the bottom of
 * the queue — they never block anything and shouldn't compete for attention. */
export function itemIsLowConfidence(item: InboxItem): boolean {
  return item.kind === "gap" && item.gap.confidence === "weak";
}

function gapAmountMsat(gap: CustodyGap): bigint {
  try {
    return BigInt(gap.source_total_msat);
  } catch {
    return 0n;
  }
}

function confidenceRank(item: InboxItem): number {
  if (item.kind === "candidate") {
    return item.candidate.confidence === "exact" ? 0 : 1;
  }
  const order = { strong: 0, moderate: 1, weak: 2 } as const;
  return order[item.gap.confidence];
}

/** Ranking: report-blocking first, then open gap questions, residual
 * follow-ups, pairing candidates, and finally weak hints; within a band the
 * engine's own confidence, then the larger amount. Mirrors the core's
 * highest-confidence / highest-amount candidate ordering. */
function itemSortKey(item: InboxItem): [number, number, number, bigint] {
  const band = itemIsLowConfidence(item)
    ? 5
    : item.kind === "gap"
      ? 1
      : item.kind === "residual"
        ? 2
        : 3;
  const amount =
    item.kind === "candidate"
      ? BigInt(Math.round(item.candidate.out_amount_msat))
      : gapAmountMsat(item.gap);
  return [itemBlocksReports(item) ? 0 : 1, band, confidenceRank(item), amount];
}

export function compareInboxItems(a: InboxItem, b: InboxItem): number {
  const ka = itemSortKey(a);
  const kb = itemSortKey(b);
  for (let i = 0; i < 3; i += 1) {
    if (ka[i] !== kb[i]) return (ka[i] as number) - (kb[i] as number);
  }
  if (ka[3] !== kb[3]) return ka[3] > kb[3] ? -1 : 1;
  return a.id.localeCompare(b.id);
}

/** Build the unified queue. Gap rows contribute an open question while they
 * need review (or conflict), and a residual follow-up once a bridge resolved
 * them but left an unclassified remainder. Everything else (resolved,
 * dismissed, reopen/revise corrections) lives in History / Advanced. */
export function buildInboxItems(
  gaps: readonly CustodyGap[],
  candidates: readonly InboxCandidate[],
): InboxItem[] {
  const items: InboxItem[] = [];
  for (const gap of gaps) {
    const mode = custodyGapActionMode(gap);
    if (mode === "create") {
      items.push({ kind: "gap", id: `gap:${gap.gap_id}`, gap });
    } else if (shouldOfferResidualClassification(gap)) {
      items.push({ kind: "residual", id: `residual:${gap.gap_id}`, gap });
    }
  }
  const seen = new Set<string>();
  for (const candidate of candidates) {
    const id = candidateItemId(candidate);
    if (seen.has(id)) continue;
    seen.add(id);
    items.push({ kind: "candidate", id, candidate });
  }
  return items.sort(compareInboxItems);
}

export function filterInboxItems(
  items: readonly InboxItem[],
  filter: InboxFilter,
): InboxItem[] {
  if (filter === "blocking") return items.filter(itemBlocksReports);
  if (filter === "suggested") return items.filter(itemIsSuggested);
  return [...items];
}

export interface InboxCounts {
  open: number;
  blocking: number;
  suggested: number;
  lowConfidence: number;
}

export function countInboxItems(items: readonly InboxItem[]): InboxCounts {
  return {
    open: items.length,
    blocking: items.filter(itemBlocksReports).length,
    suggested: items.filter(itemIsSuggested).length,
    lowConfidence: items.filter(itemIsLowConfidence).length,
  };
}

/** Distinct tax years across all report-blocking items, ascending — the
 * header's "blocks your 2024 report" phrasing. */
export function blockedYears(items: readonly InboxItem[]): number[] {
  const years = new Set<number>();
  for (const item of items) {
    if (item.kind === "candidate") continue;
    for (const year of item.gap.downstream.affected_years) years.add(year);
  }
  return [...years].sort((a, b) => a - b);
}

/** Presentation type of a candidate: same-asset moves are transfers, known
 * Bitcoin layer transitions stay Bitcoin, the rest are taxable-leaning swaps.
 * Mirrors `candidatePairType` in SwapMatching.tsx. */
const LAYER_TRANSITION_KINDS = new Set([
  "chain-swap",
  "peg-in",
  "peg-out",
  "reverse-submarine-swap",
  "submarine-swap",
  "swap-refund",
]);

export type CandidatePresentation = "transfer" | "layer-transition" | "swap";

export function candidatePresentation(
  candidate: InboxCandidate,
): CandidatePresentation {
  if (candidate.out_asset.toUpperCase() === candidate.in_asset.toUpperCase()) {
    return "transfer";
  }
  if (
    candidate.candidate_type === "transfer" ||
    LAYER_TRANSITION_KINDS.has(candidate.default_kind)
  ) {
    return "layer-transition";
  }
  return "swap";
}

/** Existing, already-translated rationale sentences from the review
 * namespace — one plain-language line per match method. */
export const CANDIDATE_WHY_KEYS = {
  payment_hash: "swap.detail.rationalePaymentHash",
  provider_swap_id: "swap.detail.rationaleProviderEvidence",
  heuristic: "swap.detail.rationaleHeuristic",
  htlc_refund: "swap.detail.rationaleHtlcRefund",
  ownership_graph: "swap.detail.rationaleOwnershipGraph",
} as const satisfies Record<InboxCandidate["method"], string>;

/** The gap engine stamps many reason codes; the card leads with the most
 * decision-relevant three and tucks the rest behind the evidence disclosure. */
const REASON_PRIORITY = [
  "structured_privacy_boundary",
  "structured_samourai_wallet",
  "structured_samourai_transaction",
  "structured_samourai_policy",
  "structured_external_origin",
  "amount_coverage_high",
  "amount_coverage_partial",
  "return_exceeds_source",
  "unresolved_residual",
  "wallet_transition",
  "split_source",
  "split_return",
  "long_horizon",
];

export function topReasonCodes(gap: CustodyGap, limit = 3): string[] {
  const ranked = [...gap.reason_codes].sort((a, b) => {
    const ia = REASON_PRIORITY.indexOf(a);
    const ib = REASON_PRIORITY.indexOf(b);
    return (ia === -1 ? REASON_PRIORITY.length : ia) -
      (ib === -1 ? REASON_PRIORITY.length : ib);
  });
  return ranked.slice(0, limit);
}
