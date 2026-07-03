# Tax and Journals Reference

Kassiber separates raw transaction storage from processed journal state. Reports should only be trusted after `journals process` has been run on the current data.

## Tax policies

Books carry tax defaults through the internal `profile` row:

- `tax_country`
- `tax_long_term_days` (generic policy only; Austrian Altbestand handling is wallet-specific)
- `gains_algorithm`

Current policies:

- `generic` -> RP2-backed lot accounting
- `at` -> RP2-backed Austrian accounting through the Kassiber-maintained fork at [bitcoinaustria/rp2](https://github.com/bitcoinaustria/rp2), with moving-average defaults for new wallets plus Kassiber-side normalization and current disposal-category / Kennzahl mapping

## Journal processing

Run:

```bash
python3 -m kassiber journals process
```

Important behavior:

- both `generic` and `at` policies currently run through RP2
- Austrian books (`tax_country=at`) use rp2's Austrian country plugin while Kassiber keeps the normalization, provenance, transfer-preparation, and current report-mapping layer
- cost basis is pooled per asset across all wallets in one set of books
- self-transfers between user-owned wallets become RP2 `IntraTransaction` moves when Kassiber can prove the relationship
- explicit inbound `kind` values such as `income`, `interest`, `staking`, `mining`, `airdrop`, `hardfork`, `wages`, `lending_interest`, and `routing_income` are promoted into RP2 earn-like receipts; unlabeled inbound rows stay conservative and process as `BUY`
- missing or ambiguous tax inputs quarantine instead of being silently guessed

After any transaction change, metadata change, exclusion change, transfer pair change, quarantine resolution, rate sync, or manual rate override, journals must be reprocessed before reports are trusted again.

## Transfers

Cross-wallet self-transfers are auto-detected when both legs share the same
on-chain `txid` and asset. Those deterministic same-chain moves are kept out of
the swap review queue; `transfers suggest` is for Lightning/Liquid layer hops
and other pairs that need review.

### Self-transfer derivation

Same-`txid` matching alone misses the cases users hit — a destination wallet
that wasn't synced for the period (no inbound row), CSV imports whose `txid`
columns don't line up, and one spend fanning out to several owned wallets.
Kassiber closes these with two extra derivation passes during
`journals process`; both **supplement** same-`txid` matching and never overrule
a manual pair. A derived move carries its basis across with no disposal, and is
tagged `ownership_derived` in the transfer audit (its journal entry reads
"proven by address ownership").

1. **Address-ownership deriver** (Bitcoin base layer). For an on-chain outbound
   whose full `vin`/`vout` are stored in `raw_json`, each output's script is
   classified against the profile-wide ownership index, restricted to the source
   wallet's `(chain, network)`. An output paying an address owned by another of
   your wallets is a self-transfer leg — proven from the graph, no heuristic. It
   reuses a recorded destination row on a shared `txid`, synthesizes the inbound
   leg for a true sync gap, and decomposes a 1→N fan-out. Anything it cannot
   prove safely (multi-wallet-input consolidation, an ambiguous destination, a
   stale-RBF amount mismatch, an output owned by two wallets) is **left on its
   existing path and flagged for review** — never mis-booked.

2. **Recorded fan-out decomposer** (chain-agnostic). When a 1→N self-transfer's
   legs were *all* synced but there is no readable graph — **Liquid** (output
   amounts are confidential, so the stored record carries no per-output graph)
   or a graphless CSV import — the rows themselves are enough: rows sharing one
   `(external_id, asset)` across two or more of your wallets are all owned, and
   the amounts conserve (`out.amount == Σ in.amount`). A single outbound fanning
   to ≥2 recorded destinations is decomposed into per-leg moves. Multi-source
   consolidations and any group whose amounts don't fully conserve (a
   destination wasn't synced) are left to the `owned_fanout_unresolved`
   quarantine.

**Scope.** Graph-based ownership derivation is **Bitcoin base layer only** —
Liquid amounts are confidential, so a Liquid spend can prove a self-transfer
only when every destination is also recorded (pass 2). A Liquid move with an
unsynced destination stays on the review path. BTC↔L-BTC pegs, Boltz swaps, and
submarine swaps are still Bitcoin movements for accounting, but the ledger keeps
`BTC` and `LBTC` separate for rail visibility; pair those with `transfers pair`.

Both passes are only as complete as the ownership index and the recorded rows. A
fan-out that is *partially* resolvable — pass 1 proves some legs from the graph
but a destination's address is outside the index's scan depth — degrades to the
existing behavior for the unresolved remainder (a disposal at the source plus a
fresh-basis acquisition at the destination), not a carried-basis move. Totals
stay correct; widen the wallet's scan depth or pair the leg manually to fix the
basis.

**Caveat — unique wallet labels.** Reports key holdings by wallet *label*. Two
wallets in one profile sharing a label merge their balances, and a derived move
routed by label can be attributed to the wrong one (totals stay correct).
`journals process` surfaces a `duplicate_wallet_label` warning; rename them.

Reports do not auto-detect or auto-pair reviewed Bitcoin swaps or other
cross-asset swaps. If you have BTC ↔ LBTC peg-ins / peg-outs, Boltz swaps, or
submarine swaps where both legs are yours, pair those legs before trusting
`journals process` and downstream reports. Swap rails can also route ordinary
payments or receipts; leave those one-sided or counterparty-owned flows unpaired
so they keep their normal payment/receipt treatment.

`transfers suggest` can surface exact matches from Lightning `payment_hash`,
redacted provider/client `swap_id` metadata, or an on-chain HTLC refund spend.
Boltz v2 cooperative Taproot key-path spends are intentionally not identifiable
from chain data alone; without metadata they remain heuristic/manual candidates.
Review blocking should stay scoped to swap-shaped unresolved flows, not ordinary
unpaired outbounds that are real payments or disposals.

When that signal is missing, you can pair them manually:

```bash
python3 -m kassiber transfers pair \
  --tx-out <OUT_TRANSACTION_ID> \
  --tx-in <IN_TRANSACTION_ID> \
  --kind manual \
  --policy carrying-value

python3 -m kassiber transfers list
python3 -m kassiber transfers unpair --pair-id <PAIR_ID>
```

Current rules:

- same-asset manual pairs support `--policy carrying-value`, including
  same-wallet failed-swap refunds where the send and refund have different
  transaction ids
- reviewed `coinjoin` pairs are same-asset carrying-value links for manually
  accepted ownership hops when descriptor history is incomplete; they preserve
  basis but do not prove the full privacy graph or counterparty set
- same-asset `--policy taxable` is rejected; leave those legs unpaired if you want normal SELL + BUY treatment
- cross-asset pairs are always stored for audit
- cross-asset `--policy carrying-value` is supported for Austrian books (`tax_country=at`): Kassiber emits reviewed swap markers, then rp2's native Austrian multi-asset hook carries basis
- cross-asset `--policy taxable` keeps the normal SELL + BUY treatment
- non-Austrian books still reject cross-asset `--policy carrying-value`
- cross-asset swaps are never auto-paired from time / amount heuristics during report generation; use `transfers pair` when those links matter for tax treatment
- swap-routed payments or receipts should stay unpaired unless both legs are known owned-wallet legs of the same user

Manual pairs override auto-detection.

## Quarantines

Inspect quarantined transactions:

```bash
python3 -m kassiber journals quarantined
python3 -m kassiber journals quarantine show --transaction <TRANSACTION_ID>
```

Typed resolution paths:

```bash
python3 -m kassiber journals quarantine resolve price-override \
  --transaction <TRANSACTION_ID> --fiat-rate 50000

python3 -m kassiber journals quarantine resolve exclude \
  --transaction <TRANSACTION_ID>

python3 -m kassiber journals quarantine clear \
  --transaction <TRANSACTION_ID>
```

Quarantine causes typically include:

- missing spot price
- missing cost basis
- insufficient lots
- ambiguous or unsupported tax semantics
- `owned_fanout_unresolved` — a fan-out / consolidation across owned wallets that
  no derivation pass could resolve (e.g. a destination wasn't synced)
- `ownership_transfer_*` — the address-ownership deriver proved a self-transfer
  but could not split it safely; review and pair manually, sync the destination,
  or review the consolidation:
  - `_destination_ambiguous` — a recorded inbound could be this leg or an
    unrelated same-value receipt (no shared `txid`)
  - `_source_ambiguous` — more than one wallet funded the spend (the per-wallet
    fee is unreliable)
  - `_amount_mismatch` — the parsed graph and the recorded amount disagree
    (stale RBF / re-org `raw_json`)
  - `_ambiguous_output` — an output is owned by two different wallets
  - `_destination_missing_ref` — the destination wallet has no account ref

## Rates and tax input quality

If transactions do not already include usable fiat pricing, Kassiber first tries to fill them from the local rates cache during journal processing. When a transaction has a known `confirmed_at` timestamp, Kassiber prices from that confirmation time; otherwise it falls back to `occurred_at`. `LBTC` / `L-BTC` transactions use the BTC fiat rate because Liquid Bitcoin is pegged one-to-one with BTC.

Pricing now carries provenance alongside the legacy numeric fields: source kind,
provider, pair, source timestamp, fetched timestamp, granularity, method, and
quality. Imported source prices can outrank cache-derived FMV.

Daily or otherwise coarse provider fallback is **accepted by default**: the event
is booked at the coarse spot price (and flagged non-blockingly in the UI), so it
flows into the capital-gains / E1kv numbers without manual intervention. The
coarse provenance is kept on the transaction for audit context. Coarse pricing is
only held back with `pricing_review_required` when a book opts into strict review:

```bash
# require manual review of coarse-priced events for this book (default: accept)
python3 -m kassiber profiles set --profile main --require-coarse-review
# return to accepting coarse prices
python3 -m kassiber profiles set --profile main --no-require-coarse-review
```

Useful commands:

```bash
python3 -m kassiber rates pairs
python3 -m kassiber rates sync --pair BTC-USD --days 30
python3 -m kassiber rates set BTC-EUR 2026-01-01T00:00:00Z 95000
python3 -m kassiber rates set BTC-EUR 2026-01-01T12:34:00Z 95000 --granularity exact
python3 -m kassiber rates latest BTC-EUR
```

Reports still use stored transaction and journal pricing rather than querying the rates cache live.

## Austrian notes

Current Austrian status:

- Austrian books process through rp2's `AT` country plugin via the shared RP2 adapter
- Kassiber keeps normalization, provenance capture, transfer preparation, reviewed swap-marker wiring, and current disposal-category / Kennzahl mapping
- Austrian cross-asset `--policy carrying-value` pairs are supported and feed rp2's native Austrian multi-asset carry path
- Austrian E 1kv export is available through `reports austrian-e1kv`,
  `reports export-austrian-e1kv-pdf`, `reports export-austrian-e1kv-xlsx`,
  and `reports export-austrian-e1kv-csv`; `reports austrian-tax-summary` and
  `reports export-austrian` are friendlier aliases for the same annual Austrian
  handoff, including outside-E-1kv rows such as KZ 801 when present
- Austrian output should remain review-gated; Kassiber is not tax advice

The E 1kv export is annual and review-oriented:

```bash
python3 -m kassiber --machine reports austrian-e1kv --year 2024
python3 -m kassiber --machine reports austrian-tax-summary --year 2024
python3 -m kassiber --format csv --output e1kv-2024.csv reports austrian-e1kv --year 2024
python3 -m kassiber reports export-austrian-e1kv-pdf --year 2024 --file e1kv-2024.pdf
python3 -m kassiber reports export-austrian --year 2024 --file austria-2024.pdf
python3 -m kassiber reports export-austrian-e1kv-xlsx --year 2024 --file e1kv-2024.xlsx
python3 -m kassiber reports export-austrian-e1kv-csv --year 2024 --dir e1kv-2024-csv
```

Kassiber currently maps crypto rows to the ausländisch / self-custody
Kennzahlen 172, 174, and 176. Domestic-provider and withheld-KESt rows such as
171, 173, or 175 need structured provider metadata before Kassiber can populate
them. The structured JSON report includes active Steuerbericht-style handoff sections
across 1.x-4.x so unsupported areas such as margin/derivatives, gifts, lost coins,
commercial mining, and minting are visible as zero-value placeholders instead
of disappearing from the handoff. The PDF export renders those same real
Kassiber rows as a styled Austrian report with summary/detail pages, holdings,
Besonderheiten, explanations, a landscape transaction appendix, a
FinanzOnline-style Kennzahl summary, and FAQ.
The XLSX export mirrors that structure as an accountant-facing workbook with
an `Übersicht` sheet, separate numbered tabs such as `1.1.`, `2.1.`, and
`3.3.`, plus an `Erläuterungen zum Steuerreport` notes sheet.
The CSV bundle mirrors the same layout as separate files so each section can
keep its own table headers instead of flattening every section into one lossy
CSV shape.

See [../plan/06-austrian-tax-engine.md](../plan/06-austrian-tax-engine.md) for the broader design and remaining Austrian backlog, plus [../austrian-handoff.md](../austrian-handoff.md) for the current marker / carry-basis contract.
