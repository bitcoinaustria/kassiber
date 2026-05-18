# Lightning data — what to store, what to discard

Lightning nodes expose much more operationally than on-chain wallets do.
Kassiber's job is local-first Bitcoin accounting and tax reporting; the
node's API surface contains everything from preimages to onion hop lists
that has zero tax value and meaningful privacy cost if the local database
leaks. This document is the discard policy adapters under
[`kassiber/core/lightning/`](../../kassiber/core/lightning/) must apply
*before* they fill the `NodeSnapshot` shapes that cross the daemon
boundary.

The guiding principle:

> **Store the minimum that answers the tax question for you, plus the
> minimum that lets you defend that number under audit, and discard
> everything whose only purpose would be to deanonymize someone else.**

## Tier 1 — never store (drop at the adapter boundary)

These fields have **zero tax value**. They are discarded at the adapter
boundary, never persisted to SQLite, never present on a `NodeSnapshot`
field, never logged, never in diagnostics bundles.

- **Preimages** (`payment_preimage`). Proof-of-payment material; a leak
  lets anyone replay invoices.
- **Payment secrets** (`payment_secret`). Same.
- **Full encoded bolt11 strings**. The string bundles `payment_hash` +
  `payment_secret` + route hints into one replayable blob that operators
  routinely paste into support tickets. Decode it once, keep the
  decoded fields you actually need, throw the encoded form away.
- **Onion / route hop lists** from `listsendpays` / `listpays`. The
  intermediate hops are the only place outside the sender where the
  full path exists; persisting them turns the DB into a
  payment-deanonymization tool. Keep `total_amount_sat` + `total_fee_sat`
  + destination pubkey only (and only the destination if you need it).
- **Route hints decoded from received invoices**. They reveal the
  *issuer's* private-channel peers — Kassiber should not undo someone
  else's confidentiality decision.
- **`failure_source_pubkey`** from failed payment attempts. Reveals
  which nodes you tried; no tax value.
- **Macaroons, commando runes, TLS keys, Unix socket paths, RPC URLs**.
  SQLCipher-encrypted `backends.config_json` only, gated by the
  reveal-token round-trip. Never in envelopes, AI tool output, logs,
  or diagnostics bundles.

## Tier 2 — aggregate, don't itemize

Per-event records are where rich node APIs leak the most. The accounting
answer almost always wants aggregates; the per-event detail is for
audit-trail confidence and reconciliation.

- **`listforwards`** → daily-per-channel routing revenue/cost rows.
  Per-forward records are a complete log of "X paid Y through me"
  patterns. Persisting them at the day-per-channel grain answers every
  tax question. If an adapter keeps per-forward rows for audit-defense
  (Austrian record-keeping under § 132 BAO is 7 years), it MUST drop
  the peer pubkeys — channel id is enough for self-reconciliation; the
  peer pubkey is what creates the deanonymization graph.
- **Balance snapshots over time.** Daily bucket with finite retention
  (suggested: 13 months — covers a full tax cycle plus one). One row
  per sync is a payment-flow oracle. The scaffold's
  `NodeSnapshot.total_local_balance_sat` is the current snapshot; if an
  adapter persists historical balances, it must bucket.
- **Bookkeeper income (CLN `bkpr-listincome`).** Persist only rows that
  *become* wallet transactions (received invoices that are income).
  Routing fees and rebalance fees are already in the
  `NodeRoutingSnapshot` aggregate — itemizing them again as wallet
  transactions double-counts and inflates disclosure surface.
- **Payment memos and invoice descriptions.** Store when the user opts
  in for an invoice trail (they're critical for income classification:
  "Consulting Q2 invoice" vs anonymous). Treat as PII: never expose
  through AI tools, redact in shareable exports.

## Tier 3 — store with boundary redaction

Some data legitimately needs persisting but is identity-linking.

- **Operator's own node pubkey** (`NodeSnapshot.pubkey`). Needed for
  reports; tag as sensitive — never in AI tool output, never in CSV
  exports unless explicitly requested, never in diagnostics bundles.
- **Channel funding outpoints** (`NodeChannel.funding_outpoint`). Needed
  to reconcile the L1↔LN swap match (Kassiber already does this via
  `payment_hash` for Boltz/submarine swaps). Tag as identity-linking;
  diagnostics and AI tool surfaces should strip them.
- **Peer pubkeys on public channels**. Already in network gossip, so
  marginal privacy cost is near zero — keep for self-reconciliation.
- **Peer pubkeys on private channels** (`is_private=True`). The peer
  chose private gossip for a reason. Kassiber must not undo that
  decision. `NodeChannel.peer_pubkey` is intentionally
  `Optional[str]`; adapters MUST pass `None` for private channels
  unless the operator has explicitly opted in to identifying their
  private peers in the local DB.
- **Peer aliases.** User-content from gossip — treat like wallet
  labels (no AI exposure, not in shareable exports).

## Authored vs. received invoices

When an adapter ingests Lightning invoices, the privacy model differs
by direction:

- **Invoices you issued** — the route hints in the bolt11 are *your*
  private channels you chose to offer. Keep if useful for
  reconciliation; the hints don't add tax value beyond amount + memo.
- **Invoices you paid (someone else issued)** — the route hints reveal
  *someone else's* private-channel topology. Drop them entirely. Keep
  amount + memo + destination pubkey + total fee.

In both cases: drop the encoded bolt11 blob, drop the
`payment_secret`, keep the decoded structured fields you actually need
(amount, memo/description, `payment_hash`, timestamp, expiry).

## Storage envelope matters

The same data has different sensitivity depending on where it ends up:

- **SQLCipher-encrypted local DB** that never leaves the operator's
  machine: a Tier-2 itemization tradeoff trends toward "keep more for
  audit defense."
- **Backed-up DB** (cloud sync, automated backups): private-channel
  peer pubkeys, routes, and memos in a backup behave like a public
  leak the moment that storage is compromised. Trend toward "keep less."
- **Diagnostics bundle / support ticket export**: Treat as if it will
  be read by a stranger. Strip Tier 3 identifier-linking columns
  (operator pubkey, peer pubkeys, funding outpoints, payment memos)
  before serializing.

## Scaffold enforcement

The shapes in [`kassiber/core/lightning/types.py`](../../kassiber/core/lightning/types.py)
encode the policy where they can:

- `NodeChannel.peer_pubkey: str | None` — adapters must pick whether to
  surface the pubkey, with the default for private channels being
  `None`.
- `NodeForward` carries short channel ids and peer **aliases** only —
  no peer pubkeys. This is deliberate; do not add a `peer_pubkey`
  field for forwards.
- `NodeSnapshot` has no fields for preimages, payment_secrets, or
  bolt11 strings. There is nowhere for an adapter to "accidentally"
  forward them; they must be discarded at the adapter boundary.
- The `LightningAdapter` Protocol exposes only
  `fetch_node_snapshot()` — there is no `pay()`, `close()`,
  `withdraw()`, or `open()` surface, by construction.

When in doubt, discard. Adding a Tier-2 record later is a small change;
revoking it from a leaked DB is impossible.
