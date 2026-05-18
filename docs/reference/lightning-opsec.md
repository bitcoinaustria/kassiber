# Lightning data — what to store, what to discard

Lightning nodes expose much more operationally than on-chain wallets do.
Kassiber's job is local-first Bitcoin accounting and tax reporting; the
node's API surface contains everything from preimages to route hop pubkey
lists that the routine tax/report computation does not need and that
carries meaningful privacy cost if the local database leaks. This
document is the discard policy adapters under
[`kassiber/core/lightning/`](../../kassiber/core/lightning/) must apply
*before* they fill the `NodeSnapshot` shapes that cross the daemon
boundary.

The guiding principle:

> **Store the minimum that answers the tax question for you, plus the
> minimum that lets you defend that number under audit, and discard
> everything whose only purpose would be to deanonymize someone else.**

## Tier 1 — never store (drop at the adapter boundary)

These fields are **not needed for routine tax/report computation**. They
are discarded at the adapter boundary, never persisted to SQLite, never
present on a `NodeSnapshot` field, never logged, never in diagnostics
bundles.

- **Preimages** (`payment_preimage`). Proof-of-payment material; a leak
  lets a third party forge proof that they paid an invoice they did
  not pay.
- **Payment secrets** (`payment_secret`). A leak enables payment
  probing and MPP-attack vectors against the issuer.
- **Full encoded bolt11 strings**. The string bundles `payment_hash` +
  `payment_secret` + route hints into one blob that operators
  routinely paste into support tickets. Decode it once, keep the
  decoded fields you actually need, throw the encoded form away.
- **Route hop pubkey lists** from `listsendpays` / `listpays`. The
  intermediate hops are the only place outside the sender where the
  full path exists; persisting them turns the DB into a
  payment-deanonymization tool. Keep `total_amount_sat` + `total_fee_sat`
  + destination pubkey only (and only the destination if you need it).
- **Route hints decoded from received invoices**. They reveal the
  *issuer's* private-channel peers — Kassiber should not undo someone
  else's confidentiality decision.
- **`failure_source_pubkey`** from failed payment attempts. Reveals
  which nodes you tried; no contribution to routine tax computation.
- **Macaroons, commando runes, TLS keys, Unix socket paths, RPC URLs**.
  SQLCipher-encrypted `backends.config_json` only, gated by the
  reveal-token round-trip. Never in envelopes, AI tool output, logs,
  or diagnostics bundles.

### Evidence vault (future opt-in)

Operators who need proof-of-payment for legal disputes, full invoice
replay for corrupted-bookkeeper recovery, or chain-of-custody records
for audits can opt into an explicit encrypted evidence-export path
(out of scope for the scaffold; tracked as a follow-up in
[`TODO.md`](../../TODO.md)). Even then, evidence vault material must
be excluded from normal daemon, AI, and diagnostics surfaces. The
default daemon path remains the discard policy above; the evidence
vault is a separate, explicit, additive workflow.

## Tier 2 — aggregate, don't itemize

Per-event records are where rich node APIs leak the most. The accounting
answer almost always wants aggregates; the per-event detail is for
audit-trail confidence and reconciliation.

- **`listforwards`** → daily-per-channel routing revenue/cost rows.
  Per-forward records are a complete log of "X paid Y through me"
  patterns. Persisting them at the day-per-channel grain covers
  routine tax/profitability reports. Even daily granularity reveals
  weekday/weekend patterns, business hours, and holiday surges when
  combined with `short_channel_id` (which encodes the open block
  height) and capacity; operators with strict counterparty privacy
  requirements should consider weekly or monthly buckets, or accept
  that the local DB is an operational record and treat backup paths
  accordingly. Per-event retention may be legally required in some
  jurisdictions (mixed VAT regimes, EU cross-border counterparty
  classification); the default should be aggregated, and per-event
  retention should be explicit opt-in with explicit boundary
  redaction. If an adapter keeps per-forward rows for audit defense
  (Austrian record-keeping under § 132 BAO is 7 years), it MUST drop
  the peer pubkeys — channel id is enough for self-reconciliation;
  the peer pubkey is what creates the deanonymization graph.
- **Balance snapshots over time.** Daily bucket with finite retention
  (suggested: 13 months — covers a full tax cycle plus one). One row
  per sync is a payment-flow oracle. The scaffold's
  `NodeSnapshot.total_local_balance_sat` is the current snapshot; if an
  adapter persists historical balances, it must bucket. Daily buckets
  still leak weekday/weekend behavior and salary-day inflows when
  paired with capacity; weekly or monthly buckets are appropriate when
  counterparty privacy matters more than reconciliation detail.
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
  `None`. `NodeChannel.__post_init__` raises `ValueError` if
  `is_private=True` is combined with a non-`None` `peer_pubkey`, so
  adapters that forget the policy fail at construction time, not at
  wire serialization time.
- `NodeChannel.short_channel_id` and `NodeChannel.funding_outpoint`
  pass through a format-only validator on construction. The validator
  does not verify the block height or the txid exists; it rejects free
  text, JSON blobs, pubkeys, and similar smuggling attempts.
- `NodeForward` carries short channel ids and peer **aliases** only —
  no peer pubkeys. This is deliberate; do not add a `peer_pubkey`
  field for forwards. `failure_reason` is a categorical
  `NodeForwardFailureReason` Literal so adapters cannot dump raw node
  error strings (which may include `failure_source_pubkey`, payment
  hashes, or route-hint JSON) through a field that otherwise looks
  free-text.
- `NodeSnapshot` has no fields for preimages, payment_secrets, or
  bolt11 strings. There is nowhere for an adapter to "accidentally"
  forward them; they must be discarded at the adapter boundary.
- The `LightningAdapter` Protocol exposes only
  `fetch_node_snapshot()` — there is no `pay()`, `close()`,
  `withdraw()`, or `open()` surface, by construction.

### AI tool surface redaction

The daemon's AI dispatch path swaps the full payload helpers
(`snapshot_to_dict` / `LightningProfitabilityReport.to_envelope_payload`)
for redacted variants (`snapshot_to_dict_for_ai` /
`to_ai_envelope_payload`). The AI payload drops the operator's own
`pubkey`, every channel's `peerPubkey` / `peerAlias` / `shortChannelId`
/ `fundingOutpoint`, every forward's peer aliases and short channel
ids, and the per-channel covers-open-cost rows. The desktop / CLI
surface still returns the full payload — it is the operator's own
data, displayed in their own UI. The AI redacted shape applies even
when the operator owns the connection because tool transcripts are
easier to leak than the GUI.

When in doubt, discard. Adding a Tier-2 record later is a small change;
revoking it from a leaked DB is impossible.
