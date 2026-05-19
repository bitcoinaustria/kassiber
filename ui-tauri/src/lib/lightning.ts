/**
 * Shared constants/labels for Lightning surfaces.
 *
 * Mirrors `kassiber/core/lightning/profitability.py::DEFAULT_OPEN_COST_SAT`
 * and the wallet kinds tuple. When the Python default changes, update this
 * file in the same commit so the mock daemon and Reports panel agree with
 * what live adapters return.
 */

export const DEFAULT_OPEN_COST_SAT = 2_500;

// Wallet kinds the Python daemon treats as Lightning nodes. The UI
// catalog uses `core-ln` (hyphen) for display while the wallets table
// stores `coreln`; both spellings are accepted across the boundary.
export const LIGHTNING_CONNECTION_KINDS: ReadonlySet<string> = new Set([
  "core-ln",
  "coreln",
  "lnd",
  "nwc",
]);
