/**
 * Shared matching between a connection/wallet and a transaction.
 *
 * The overview transaction rows (`Tx`) and the dashboard table records only
 * carry a free-form `account` display string — not a wallet id. For ordinary
 * rows that string is the wallet label ("Cold Storage"); for transfers it is a
 * two-sided label joined with an arrow ("Cold Storage → Vault").
 *
 * The old surfaces disagreed: the Wallet Detail recent-transactions list used a
 * substring match (`account.includes(label)`), which leaked unrelated wallets
 * ("Cold" matching "Coldcard backup") and pulled in transfers by accident,
 * while the Transactions table used an exact match (`wallet === key`), which
 * silently *dropped* transfer rows that touch the wallet. This helper is the
 * single source of truth both surfaces use so they agree: a transaction belongs
 * to a connection when the connection label equals the account, or equals one
 * leg of a transfer account — matched whole-value and case-insensitively, never
 * as a substring.
 */

import type { Connection, Tx } from "@/mocks/seed";

/** Separators kassiber uses to join the two sides of a transfer account. */
const TRANSFER_SEPARATORS = /\s*(?:→|⇄|↔|⟶|->|=>)\s*/;

function normalize(value: string | null | undefined): string {
  return (value ?? "").trim().toLowerCase();
}

/**
 * Split a transfer-style account ("A → B") into its individual legs. Ordinary
 * single-wallet accounts return a one-element array; blank input returns [].
 */
export function accountLegs(account: string | null | undefined): string[] {
  return (account ?? "")
    .split(TRANSFER_SEPARATORS)
    .map((leg) => leg.trim())
    .filter((leg) => leg.length > 0);
}

/**
 * True when `label` matches `account` exactly or matches one transfer leg.
 * Whole-value, case-insensitive — never a substring match.
 */
export function accountMatchesLabel(
  account: string | null | undefined,
  label: string | null | undefined,
): boolean {
  const target = normalize(label);
  if (!target) return false;
  return accountLegs(account).some((leg) => leg.toLowerCase() === target);
}

/** Convenience wrapper for overview transaction rows and connection objects. */
export function transactionBelongsToConnection(
  tx: Pick<Tx, "account">,
  connection: Pick<Connection, "label">,
): boolean {
  return accountMatchesLabel(tx.account, connection.label);
}
