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
      candidate.default_policy === "carrying-value",
  ).slice(0, 50);
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
 * What pairing a candidate records: a move of the same asset, bitcoin
 * changing rails, or a swap into another asset. The same reading as
 * Transfers & Custody.
 */
export function moveType(candidate: TransferCandidate): "transfer" | "bitcoinSwap" | "swap" {
  if (isSameAsset(candidate)) return "transfer";
  if (candidate.candidate_type === "transfer" || BITCOIN_SWAP_KINDS.has(candidate.default_kind)) {
    return "bitcoinSwap";
  }
  return "swap";
}
