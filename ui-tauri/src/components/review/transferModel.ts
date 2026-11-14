/**
 * Transfer candidates as the Review screen decides them: one pair of
 * transactions the matcher thinks moved coins between the owner's wallets.
 * Pairing is the owner's call; a suggestion never pairs on its own.
 */

/**
 * The reason stored with a dismissal. The book keeps one language, whatever
 * the app is shown in, like the reasons stored for accepted and entered prices.
 */
export const DISMISS_REASON = "user dismissed from review queue";

export interface TransferCandidate {
  out_id: string;
  in_id: string;
  out_asset: string;
  in_asset: string;
  out_amount_msat: number;
  in_amount_msat: number;
  out_wallet_label: string;
  in_wallet_label: string;
  /** The wallet's kind, which tells the rail (chain, Lightning, Liquid); absent from older daemons. */
  out_wallet_kind?: string;
  in_wallet_kind?: string;
  out_occurred_at: string;
  in_occurred_at: string;
  confidence: "exact" | "strong";
  method: string;
  swap_fee_msat: number;
  default_kind: string;
  default_policy: string;
  candidate_type?: "transfer" | "swap";
  conflict_set_id: string;
  /** Candidates sharing a leg with this one, itself included. */
  conflict_size: number;
  /** The owner's linking rule that matches this candidate, with the terms it sets. */
  rule_match?: { rule_id: string; rule_name: string | null; kind?: string | null; policy?: string | null };
}

/** The kind and treatment linking it records: a matching rule's, else the matcher's default. */
export function candidateTerms(candidate: TransferCandidate): { kind: string; policy: string } {
  return {
    kind: candidate.rule_match?.kind || candidate.default_kind,
    policy: candidate.rule_match?.policy || candidate.default_policy,
  };
}

/**
 * A match to look at before linking: not exact, or competing with another
 * match for one of its sides. These open the receipt picker, not a one-click link.
 */
export function needsReview(candidate: TransferCandidate): boolean {
  return candidate.confidence !== "exact" || candidate.conflict_size > 1;
}

export interface TransferSuggestions {
  candidates: TransferCandidate[];
  counts: { total: number; exact: number; strong: number; conflicts: number };
}

export interface BulkPairResult {
  applied: Array<{ id: string; out_transaction_id: string; in_transaction_id: string }>;
  summary: { count: number; skipped_conflicts: number; total_swap_fee_msat: number };
}

export const candidateKey = (candidate: TransferCandidate) =>
  `${candidate.out_id}->${candidate.in_id}`;

/**
 * The candidates the exact bulk step pairs: exact evidence, no competing
 * match, and never an ownership-graph proof, which stays a one-at-a-time
 * confirmation. Taxable policies require an individual decision. The
 * review core caps each atomic plan at 50 operations.
 */
export function bulkPairable(candidates: TransferCandidate[]) {
  return candidates.filter(
    (candidate) =>
      candidate.confidence === "exact" &&
      candidate.conflict_size <= 1 &&
      candidate.method !== "ownership_graph" &&
      candidateTerms(candidate).policy === "carrying-value",
  ).slice(0, 50);
}

/** Asset codes that are bitcoin on any rail (chain, Lightning, Liquid, exchange ledgers). */
export const BITCOIN_ASSETS: ReadonlySet<string> = new Set(["BTC", "XBT", "XXBT", "LBTC", "L-BTC"]);

export const isBitcoinAsset = (asset: string) => BITCOIN_ASSETS.has(asset.trim().toUpperCase());

/**
 * Another asset on the receiving side: a swap out of bitcoin, taxable unless
 * its policy says otherwise. Bitcoin changing rails (BTC to LBTC, a peg, a
 * Lightning swap) is not one.
 */
export function isDifferentAssetMove(outAsset: string, inAsset: string, kind?: string | null): boolean {
  if (outAsset.trim().toUpperCase() === inAsset.trim().toUpperCase()) return false;
  if (isBitcoinAsset(outAsset) && isBitcoinAsset(inAsset)) return false;
  return !(kind && BITCOIN_SWAP_KINDS.has(kind));
}

/** Same asset on both legs: a move between wallets rather than a swap. */
export const isSameAsset = (candidate: TransferCandidate) =>
  candidate.out_asset.toUpperCase() === candidate.in_asset.toUpperCase();

/** Pair kinds that move bitcoin between its own rails: peg-ins, peg-outs and Lightning swaps. */
export const BITCOIN_SWAP_KINDS: ReadonlySet<string> = new Set([
  "chain-swap",
  "peg-in",
  "peg-out",
  "reverse-submarine-swap",
  "submarine-swap",
  "swap-refund",
]);

/**
 * What linking a candidate records: a move of the same asset, bitcoin
 * changing rails, or a swap into another asset. The same reading as the
 * Transfers screen.
 */
export function moveType(candidate: TransferCandidate): "transfer" | "bitcoinSwap" | "swap" {
  if (isSameAsset(candidate)) return "transfer";
  if (candidate.candidate_type === "transfer" || BITCOIN_SWAP_KINDS.has(candidate.default_kind)) {
    return "bitcoinSwap";
  }
  return "swap";
}

/** The chip on a swap into another asset, wherever such a move is listed. */
export const differentAssetBadgeClassName =
  "rounded-md border-amber-500/40 bg-amber-500/10 px-1.5 py-0 text-2xs leading-tight text-amber-800 dark:text-amber-200";
