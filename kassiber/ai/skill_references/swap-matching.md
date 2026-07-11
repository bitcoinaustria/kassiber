# Swap matching

When the user holds Bitcoin across Lightning, Liquid (LBTC), and on-chain
BTC wallets and moves funds between them, Kassiber treats those moves as
**swaps** — not as taxable disposals. The matcher pairs the two legs so
the carrying-value math applies and the user sees only the actual fee as
the real outflow.

## When to reach for it

- The user says "this Phoenix LN send and this Liquid receive are the same
  swap" or anything similar (peg, submarine swap, Boltz, Aqua, federation).
- The user has many LBTC↔BTC or LN↔Liquid pairs and wants to batch.
- A "swap" tag or note appears on transactions but the report still shows
  them as separate taxable disposals.
- Reports surface "Neu gain" / "Income receipt" on legs that the user
  insists were one swap.

## Fast paths

| User asks for... | First command |
|---|---|
| Find swap candidates | `kassiber --machine transfers suggest` |
| Preview exact auto-pairs without writing | `kassiber --machine transfers bulk-pair --confidence exact --dry-run` |
| Auto-pair all exact, non-conflicted matches | `kassiber --machine transfers bulk-pair --confidence exact` |
| Pair two specific legs manually | `kassiber --machine transfers pair --tx-out <id> --tx-in <id> --kind submarine-swap --policy carrying-value` |
| Record a direct swap payout to an external recipient | `kassiber --machine transfers payouts create --tx-out <id> --payout-asset BTC --payout-amount <btc> --payout-fiat-value <fiat> --policy carrying-value` |
| Soft-delete a pair (audit row stays) | `kassiber --machine transfers unpair --pair-id <id>` |
| Dismiss a false-positive candidate for 90 days | `kassiber --machine transfers dismiss --tx-out <id> --tx-in <id> --reason "not a swap"` |
| Total swap fees by year | `kassiber --machine reports tax-summary` — read the rows with `row_type=swap_fees_year` / `swap_fees_total` |

After every `transfers pair`, `transfers payouts create/delete`,
`transfers bulk-pair`, `transfers unpair`, or `transfers dismiss`,
re-run `kassiber --machine journals process` before trusting any report.

## Confidence ladder

`transfers suggest` emits `exact` and `strong` confidence bands; they
have different review requirements.

- **`exact`** — a deterministic whole-row link, safe to bulk-pair only when
  non-conflicted. Examples are: a native-adapter or witness-proven payment hash
  with unique 1:1 cardinality, the same Bitcoin network domain, and compatible
  whole-row amounts; an HTLC refund with
  one witness-proven canonical funding outpoint and amount coverage; provider
  evidence with a unique 1:1 key, canonical route txids, and
  explicit integer-msat principal amounts covering both complete rows, with no
  contradictory provider/id/flow/status/route aliases; or an
  ownership-graph proof whose canonical transaction scope and amounts cover the
  rows exactly.
- **`strong`** — useful but incomplete review evidence: route-only provider
  metadata, legacy/batched script hashes, amount-compatible ownership receipts,
  or different-wallet time + amount matches within the default 24h / fee band.
  Always inspect these manually; they are excluded from default exact bulk-pair.

`conflicts > 0` means two or more candidates share a leg. Each
candidate carries `conflict_set_id` plus `conflict_size` — the
cluster's cardinality stamped at match time over the FULL candidate
set. Bulk-pair, rule auto-apply, and the review surfaces all gate on
`conflict_size > 1`, so a cluster split across filters (for example the
swap vs transfer tabs, or an `asset_pair` filter that hides one
sibling) still blocks every member from bulk actions. Resolving a
conflict is manual: pair the correct candidate from the row (pairing
consumes its legs, so the losing siblings disappear on the next
suggest) or dismiss the wrong ones.

## What the matcher does NOT do

- It never hardcodes Liquid federation addresses. Peg detection is
  purely heuristic (asset + direction + amount + time window) plus the
  exact-hash path for submarine swaps.
- It does not surface deterministic same-asset self-transfers in the review
  queue. One canonical `(chain, network, txid, consensus asset identity)` scope with exact owned
  principal/fee evidence belongs to the journal self-transfer path instead.
  Arbitrary provider/import `external_id` values are never physical identity.
  Run `kassiber --machine journals transfers list` after processing to
  audit those moves.
- It never auto-pairs without explicit user opt-in (CLI flag,
  consented daemon action, or rule the user created).
- It never silently overrides pair-policy validation. Bitcoin ownership and
  matching are country-neutral; only after a link is proven does the profile's
  tax policy decide whether an unlike-asset conversion can carry value.

## Failed swaps and refunds

A swap that fails (the Lightning payment can't be made, the invoice
expires) is swept back on-chain through the HTLC's CLTV timeout branch.
It shows up as two transactions with **different** txids: an outbound
**lockup** to the swap HTLC, and a later inbound **refund** that returns
the asset minus on-chain fees. Economically nothing was disposed of —
the only cost is the miner fees. Left unpaired, the lockup books as a
phantom SELL and the refund as a phantom BUY.

Two pieces handle this:

- **Pairing is allowed, even same-wallet.** The refund normally returns
  to the funding wallet, so same-wallet same-asset pairs are accepted.
  Pair the send and refund with `--kind swap-refund --policy carrying-value`
  (any same-asset profile — this is a self-transfer, not a cross-asset
  swap, so it does not need an Austrian profile). The round trip books as
  a transfer that realizes only the fee delta; no disposal.

  ```
  kassiber transfers pair --tx-out <lockup-id> --tx-in <refund-id> \
    --kind swap-refund --policy carrying-value
  ```

- **Automatic detection from chain data.** When BTC/Liquid descriptor
  sync (esplora / electrum) sees an inbound tx whose input spends a
  Boltz v1 HTLC via the refund (timeout) branch, it records the funding
  txid it spent on `transactions.swap_refund_funding_txid`. The matcher
  pairs that refund to the outbound leg in that canonical funding route and
  surfaces it as an **exact** `swap-refund` candidate only when one refund input
  witness proves the funding outpoint and whole-row amount coverage is unique
  (method `htlc_refund`) — same-wallet
  and outside the time window included. Txid-only legacy evidence stays strong.
  Filter to just these with `transfers suggest --method htlc_refund`.

  Surfacing only — like every exact candidate it auto-pairs only via an
  explicit `transfers bulk-pair`, a rule, or a user action.

Coverage limits: the link needs on-chain witness data, so it covers
chain-synced Boltz v1 P2WSH HTLC refunds. CSV/exchange imports and Boltz
v2 Taproot cooperative refunds carry no witness, and rows synced before
the `swap_refund_funding_txid` column existed are not backfilled — those
fall back to the heuristic (different-wallet refunds inside the window)
or to manual `swap-refund` pairing.

## Direct swap payouts

Use `transfers payouts create` when there is no owned inbound leg because
the swap provider paid a recipient or exchange directly. This records the
reviewed source outbound, target asset amount, external payout id,
counterparty, fiat payout value, policy, and swap-fee delta without
creating a fake recipient wallet.

The direct payout review model is not Austrian-only: when
`payout_fiat_value` is present, it becomes the reviewed proceeds for the
taxable source-row disposal. That lets privacy-preserving provider payout
flows preserve the actual sale value even when no owned inbound leg exists.

Reviewed BTC ↔ LBTC rail changes may carry value on every profile while the
Bitcoin-rail setting is enabled. Austrian policy additionally supports
reviewed carrying-value treatment for other eligible crypto conversions.
Detection and payout evidence remain country-neutral; tax treatment is applied
only after the route is proven.

## Closing multi-wallet gaps

Use `transfers components bulk-resolve --dry-run` for 1:N, N:1, N:M,
multi-hop migrations, or missing intermediate wallets. Represent missing owned
custody with `untracked_wallet`; genuine N:M requires explicit allocations.
The equivalent chat workflow is `ui.transfers.components.list`, followed by a
`dry_run=true` call to `ui.transfers.components.bulk_resolve` and a separately
consented final write.
Activate the complete component only after every imported anchor is covered and
quantity/conversion conservation passes atomically. An unknown intermediate
wallet is missing evidence, not a network reset: known main/test/regtest/signet
domains must agree across the complete route, including separately authored
components that reuse the same placeholder. Source allocations must not occur
after their sinks.

## Swap fees as the real outflow

Carrying-value swaps preserve principal — the only thing that leaves
the user's custody is the fee delta between the two legs. The matcher
computes that delta once at pair time and persists it on
`transaction_pairs.swap_fee_msat`; direct payout reviews persist the
same delta on `direct_swap_payouts.swap_fee_msat`. Surfacing this
number is the "what actually left your custody" framing the user
typically wants.

- `kassiber --machine transfers suggest` exposes `swap_fee_msat` and
  `swap_fee` (BTC float) on every candidate.
- `kassiber --machine transfers list` shows the persisted fee on every
  active pair.
- `kassiber --machine reports tax-summary` aggregates per-year and
  grand total into rows with `row_type=swap_fees_year` and
  `row_type=swap_fees_total`.

A negative `swap_fee_msat` is an anomaly (the inbound exceeded the
outbound). The matcher rejects those candidates in the strong heuristic
band; if you see one persisted, the pair was created manually with the
wrong legs — unpair and re-pair.

## Auto-pair rules

When the same swap shape repeats, the user can promote it to a rule:

```
kassiber transfers rules create \
  --name "Phoenix to Liquid" \
  --predicate '{"out_wallet_kind":"phoenix","in_wallet_kind":"descriptor",
                "in_asset":"LBTC","max_fee_pct":0.01,
                "min_confidence":"strong"}' \
  --kind submarine-swap \
  --policy carrying-value
```

Rules auto-apply to solo (non-conflicted) candidates that match every
non-empty predicate field. Conflict clusters are never auto-paired —
the user always disambiguates. Preview with `transfers rules apply --dry-run`,
then apply with `transfers rules apply`;
list with `transfers rules list`;
toggle with `transfers rules enable|disable --rule-id <id>`; delete
with `transfers rules delete --rule-id <id>`.

## Saved review-queue filters

`views {list,create,delete}` persists filter snapshots scoped to a
surface (the matcher uses `swap_candidates`). The UI renders these as
header chips so heavy users can switch between "Boltz pegouts" and
"Phoenix LN→Liquid awaiting review" with one click.

## Boundary with the tax engine

- Kassiber owns the country-neutral evidence graph: ownership detection,
  confidence scoring, conflict clusters, fee evidence, dismissal lifecycle,
  and rule application.
- rp2 owns same-asset MOVE (`IntraTransaction`) and disposal-category
  bucketing. Kassiber's generic policy can carry reviewed BTC ↔ LBTC Bitcoin
  exposure while enabled; the AT plugin additionally handles eligible other
  reviewed multi-asset carry treatment.

## HTLC payment-hash extraction

Where the matcher's exact-match path applies:

- **Phoenix CSV imports** — every Lightning row already exposes
  `payment_hash` in the source. The importer promotes it to
  `transactions.payment_hash` so the matcher can use it directly.
- **BTC + Liquid descriptor sync (esplora / electrum)** — the parser
  opportunistically extracts a preimage from claim-tx witnesses and
  records the resulting `payment_hash` only when the transaction has exactly
  one input and that claim names a canonical funding outpoint, with
  `payment_hash_source = "chain_script_unique_outpoint"`. Boltz v1 P2WSH
  HTLCs are covered (both submarine and reverse variants). Batched claims and
  legacy unversioned `chain_script` rows remain strong/manual evidence; they
  never become exact whole-row matches by selecting the first witness.
- **Boltz v2 Taproot cooperative spends** reveal nothing on-chain
  (key-path Schnorr signature only), so those swaps fall through to
  the heuristic match by physics, not by deferral.
- **Failed-swap refunds** take the HTLC timeout branch and reveal no
  preimage, so there is no `payment_hash`. Sync instead records the
  funding txid the refund spent on `transactions.swap_refund_funding_txid`.
  Exact promotion additionally requires one witness-proven input and full-row
  amount coverage; txid/outpoint metadata alone stays strong (see
  [Failed swaps and refunds](#failed-swaps-and-refunds)).

LND and Core Lightning already feed source-qualified payment hashes into this
boundary; future NWC or other Bitcoin-layer adapters must emit the same typed
identity, amount, conservation, and evidence-grade facts rather than adding
country-specific matching logic.
