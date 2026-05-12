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
| Auto-pair all exact (payment_hash) matches | `kassiber --machine transfers bulk-pair --confidence exact` |
| Pair two specific legs manually | `kassiber --machine transfers pair --tx-out <id> --tx-in <id> --kind submarine-swap --policy carrying-value` |
| Soft-delete a pair (audit row stays) | `kassiber --machine transfers unpair --pair-id <id>` |
| Dismiss a false-positive candidate for 90 days | `kassiber --machine transfers dismiss --tx-out <id> --tx-in <id> --reason "not a swap"` |
| Total swap fees by year | `kassiber --machine reports tax-summary` — read the rows with `row_type=swap_fees_year` / `swap_fees_total` |

After every `transfers pair`, `transfers bulk-pair`, `transfers unpair`,
or `transfers dismiss`, re-run `kassiber --machine journals process`
before trusting any report.

## Confidence ladder

`transfers suggest` emits two confidence bands; they have different
review requirements.

- **`exact`** — both legs share a Lightning ``payment_hash``. This is
  cryptographic identity across the swap; bulk-pair these without per-row
  review. Method is `payment_hash`.
- **`strong`** — different wallets, opposite directions, time delta
  within the window (default 24h), and `|out_amount - in_amount|` sits
  below the fee threshold (`max(1% of out, 2500 sats)`). Method is
  `heuristic`. Always eyeball before pairing — the user has to confirm.

`conflicts > 0` means two or more candidates share a leg. Bulk-pair
intentionally skips conflict clusters; the user must disambiguate
manually.

## What the matcher does NOT do

- It never hardcodes Liquid federation addresses. Peg detection is
  purely heuristic (asset + direction + amount + time window) plus the
  exact-hash path for submarine swaps.
- It never auto-pairs without explicit user opt-in (CLI flag,
  consented daemon action, or rule the user created).
- It never silently overrides the existing `transfers pair` validation
  rules: cross-asset `policy=carrying-value` still requires an Austrian
  profile; same-asset `policy=taxable` is still rejected.

## Swap fees as the real outflow

Carrying-value swaps preserve principal — the only thing that leaves
the user's custody is the fee delta between the two legs. The matcher
computes that delta once at pair time and persists it on
`transaction_pairs.swap_fee_msat`. Surfacing this number is the
"what actually left your custody" framing the user typically wants.

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
the user always disambiguates. Apply with `transfers rules apply`;
list with `transfers rules list`;
toggle with `transfers rules enable|disable --rule-id <id>`; delete
with `transfers rules delete --rule-id <id>`.

## Saved review-queue filters

`views {list,create,delete}` persists filter snapshots scoped to a
surface (the matcher uses `swap_candidates`). The UI renders these as
header chips so heavy users can switch between "Boltz pegouts" and
"Phoenix LN→Liquid awaiting review" with one click.

## Boundary with the tax engine

- Kassiber owns: pair detection, confidence scoring, conflict clusters,
  fee computation, dismissal lifecycle, rule application.
- rp2 owns: same-asset MOVE (`IntraTransaction`), AT cross-asset
  carrying-value math (via `compute_tax_for_assets` on the AT plugin),
  disposal category bucketing.
- For non-AT profiles, cross-asset carrying-value is still unsupported
  in rp2 — those pairs surface in `cross_asset_pairs` audit but fall
  through to SELL+BUY in the journal. `transfers pair` rejects
  `policy=carrying-value` on non-AT cross-asset pairs with a clear
  validation envelope.

## HTLC payment-hash extraction

Where the matcher's exact-match path applies:

- **Phoenix CSV imports** — every Lightning row already exposes
  `payment_hash` in the source. The importer promotes it to
  `transactions.payment_hash` so the matcher can use it directly.
- **BTC + Liquid descriptor sync (esplora / electrum)** — the parser
  opportunistically extracts a preimage from claim-tx witnesses and
  records the resulting `payment_hash` with
  `payment_hash_source = "chain_script"`. Boltz v1 P2WSH HTLCs are
  covered (both submarine and reverse variants).
- **Boltz v2 Taproot cooperative spends** reveal nothing on-chain
  (key-path Schnorr signature only), so those swaps fall through to
  the heuristic match by physics, not by deferral.

The exact-match path is also future-proofed for `coreln`, `lnd`, and
`nwc` adapters once they sync — they all expose `payment_hash` on
Lightning rows, and the importer normaliser already accepts the field.
