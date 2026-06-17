/**
 * Reconcile a wallet's two balance views on the Wallet Detail screen.
 *
 * The "Balance" tile sums imported transactions (`connection.balance`), while
 * the UTXO inventory panel shows the watch-only on-chain coin set. For a
 * fully-synced watch-only wallet these are the same number; a gap usually
 * means a stale sync, or transactions that are excluded / quarantined and so
 * never reached the summed balance. Surfacing the comparison turns a confusing
 * "₿1.248 here, ₿0.156 there" into an explicit reconciled / needs-attention
 * signal.
 *
 * Kept as a pure, structurally-typed helper (no component imports) so the
 * float-BTC → satoshi conversion is unit-testable.
 */

const RECONCILABLE_ASSETS = new Set(["BTC", "LBTC", "L-BTC"]);
/** Float BTC × 1e8 can drift a fraction; absorb sub-satoshi rounding noise. */
const RECONCILE_TOLERANCE_SAT = 1;

interface ReconcilableInventory {
  totals?: Array<{ asset?: string; amount_sat?: number }>;
  support?: { supported?: boolean };
  freshness?: { last_synced_at?: string | null; last_seen_at?: string | null };
}

export interface BalanceReconciliation {
  /** Whether this source exposes a UTXO inventory that can be reconciled. */
  available: boolean;
  importedSat: number;
  utxoSat: number;
  /** importedSat − utxoSat (positive = imported balance exceeds on-chain). */
  deltaSat: number;
  reconciled: boolean;
  lastSyncedAt: string | null;
}

export function buildBalanceReconciliation(
  importedBtc: number,
  inventory: ReconcilableInventory | null | undefined,
): BalanceReconciliation {
  const importedSat = Math.round((importedBtc ?? 0) * 1e8);
  const available =
    Boolean(inventory) && inventory?.support?.supported !== false;
  const utxoSat = (inventory?.totals ?? [])
    .filter((total) =>
      RECONCILABLE_ASSETS.has((total.asset ?? "").toUpperCase()),
    )
    .reduce((sum, total) => sum + (total.amount_sat ?? 0), 0);
  const deltaSat = importedSat - utxoSat;
  return {
    available,
    importedSat,
    utxoSat,
    deltaSat,
    reconciled: Math.abs(deltaSat) <= RECONCILE_TOLERANCE_SAT,
    lastSyncedAt:
      inventory?.freshness?.last_synced_at ??
      inventory?.freshness?.last_seen_at ??
      null,
  };
}
