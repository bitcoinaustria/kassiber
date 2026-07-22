/**
 * Pure model for the Custody Inbox — the queue of custody *questions*:
 * missing-wallet gap candidates from ``ui.custody.gaps.list`` and their
 * residual follow-ups. High-volume transfer/swap *matching* is deliberately
 * NOT part of the inbox — it lives in the dedicated pairing queue (the
 * "Moves & swaps" tab), which is a different activity: fast, evidence-backed
 * confirms rather than careful judgment calls.
 */

import {
  custodyGapActionMode,
  shouldOfferResidualClassification,
  type CustodyGap,
} from "../custodyGapsModel";

export type InboxItem =
  | { kind: "gap"; id: string; gap: CustodyGap }
  /** A resolved bridge that left an unclassified remainder — the follow-up
   * question ("where did the missing amount go?"). */
  | { kind: "residual"; id: string; gap: CustodyGap };

export type InboxFilter = "all" | "blocking" | "suggested";

/** A gap whose resolution re-opens later disposals/tax years gates correct
 * reports — those questions surface first. */
export function itemBlocksReports(item: InboxItem): boolean {
  return (
    item.gap.downstream.affected_disposals > 0 ||
    item.gap.downstream.affected_years.length > 0
  );
}

/** "Suggested" = the engine itself considers this safe to propose
 * (promotion-eligible structured evidence). */
export function itemIsSuggested(item: InboxItem): boolean {
  return item.gap.promotion_eligible;
}

export function itemHasCompetingEvidence(item: InboxItem): boolean {
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

const CONFIDENCE_ORDER = { strong: 0, moderate: 1, weak: 2 } as const;

/** Ranking: report-blocking first, then open questions before residual
 * follow-ups, weak hints last; within a band the engine's own confidence,
 * then the larger amount. Mirrors the core's candidate ordering. */
function itemSortKey(item: InboxItem): [number, number, number, bigint] {
  const band = itemIsLowConfidence(item) ? 5 : item.kind === "gap" ? 1 : 2;
  return [
    itemBlocksReports(item) ? 0 : 1,
    band,
    CONFIDENCE_ORDER[item.gap.confidence],
    gapAmountMsat(item.gap),
  ];
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

/** Build the question queue. Gap rows contribute an open question while they
 * need review (or conflict), and a residual follow-up once a bridge resolved
 * them but left an unclassified remainder. Everything else (resolved,
 * dismissed, reopen/revise corrections) lives in History / Advanced. */
export function buildInboxItems(gaps: readonly CustodyGap[]): InboxItem[] {
  const items: InboxItem[] = [];
  for (const gap of gaps) {
    const mode = custodyGapActionMode(gap);
    if (mode === "create") {
      items.push({ kind: "gap", id: `gap:${gap.gap_id}`, gap });
    } else if (shouldOfferResidualClassification(gap)) {
      items.push({ kind: "residual", id: `residual:${gap.gap_id}`, gap });
    }
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
 * header's "blocks your reports" phrasing. */
export function blockedYears(items: readonly InboxItem[]): number[] {
  const years = new Set<number>();
  for (const item of items) {
    for (const year of item.gap.downstream.affected_years) years.add(year);
  }
  return [...years].sort((a, b) => a - b);
}

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
