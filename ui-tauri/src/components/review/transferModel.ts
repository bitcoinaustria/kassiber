/**
 * Transfer candidates as the Review screen decides them: one pair of
 * transactions the matcher thinks moved coins between the owner's wallets.
 * Pairing is the owner's call; a suggestion never pairs on its own.
 */

export interface TransferCandidate {
  out_id: string;
  in_id: string;
  out_asset: string;
  in_asset: string;
  out_amount_msat: number;
  in_amount_msat: number;
  out_wallet_label: string;
  in_wallet_label: string;
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
 * confirmation. Mirrors the daemon's `bulk_pair` selection, which decides.
 */
export function bulkPairable(candidates: TransferCandidate[]) {
  return candidates.filter(
    (candidate) =>
      candidate.confidence === "exact" &&
      candidate.conflict_size <= 1 &&
      candidate.method !== "ownership_graph",
  );
}

/** Same asset on both legs: a move between wallets rather than a swap. */
export const isSameAsset = (candidate: TransferCandidate) =>
  candidate.out_asset.toUpperCase() === candidate.in_asset.toUpperCase();
