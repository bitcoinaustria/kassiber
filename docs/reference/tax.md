# Tax and Journals Reference

Kassiber separates raw transaction storage from processed journal state. Reports should only be trusted after `journals process` has been run on the current data.

## Tax policies

Books carry tax defaults through the internal `profile` row:

- `tax_country`
- `tax_long_term_days` (generic policy only; Austrian Altbestand handling is wallet-specific)
- `gains_algorithm`
- `bitcoin_rail_carrying_value` (default on; controls the generic default treatment for BTC-equivalent rail changes)
- `cost_basis_pool_scope` (country-policy allowlisted; every current policy permits only `global`)

Current policies:

- `generic` -> lot accounting in Kassiber's [tax engine](tax-engine.md)
- `at` -> Austrian accounting in the same engine (Austrian normalization, the regime tracker, and moving-average defaults for new wallets), with Python keeping the current disposal-category / Kennzahl mapping

## Implementation boundary

[custody_journal.py](../../kassiber/core/custody_journal.py) composes imported
observations, observer authority, and authored custody evidence into canonical
decisions, issues, lineage, and finalized tax inputs. CLI handlers call this
service rather than assembling a second interpreter.

`KassiberTaxEngine` (`kassiber/core/engines/ledger.py`) consumes a
`FinalizedTaxProjection`. Both generic and Austrian books use it; Austrian
books select the engine's Austrian country. Python sends the projection's rows
as typed facts and renders the engine's journal ledger. Kassiber's
[tax engine](tax-engine.md) owns country defaults, normalization, the Austrian
regime tracker, the balance and quarantine gates, carry math, lot computation,
and journal composition, and reproduces the phase-3 adapter on RP2 exactly; RP2
remains only as the tests' parity oracle. The engine owns the
marker/quarantine contract and the disposal categories; Python maps the
categories into report buckets. See [the Austrian handoff](../austrian-handoff.md).

Engine calls pool each asset across the wallets of a profile, so a same-asset
move can carry basis between owned wallets. Wallet identity is
preserved through each entry's exchange label and per-wallet balances.
Per-wallet portfolio basis is an allocation at the asset's average residual
basis, not a claim about which physical lots remain in a wallet.

## Journal processing

Run:

```bash
python3 -m kassiber journals process
```

Important behavior:

- both `generic` and `at` policies run through Kassiber's [tax engine](tax-engine.md), and journal entries record `"engine": "kassiber_tax"` in their calculation metadata
- Austrian books (`tax_country=at`) use the engine's Austrian country rules while Kassiber keeps the provenance, transfer-preparation, and current report-mapping layer
- cost basis is currently pooled per asset across all wallets in one set of
  books; narrower scopes fail closed unless that country policy explicitly
  advertises and implements them
- self-transfers between user-owned wallets become same-asset moves (`transfer_out`/`transfer_in` journal legs, and a `transfer_fee` entry for a fee) when Kassiber can prove the relationship
- explicit inbound `kind` values such as `income`, `interest`, `staking`, `mining`, `airdrop`, `hardfork`, `lending_interest`, and `routing_income` are promoted into earn-like receipts; valued `wages` preserve their source provenance but process as an ordinary `BUY` acquisition with the reviewed EUR value as basis, while wage-tax reporting remains outside Kassiber; unlabeled inbound rows stay conservative and process as `BUY`
- missing or ambiguous tax inputs quarantine instead of being silently guessed

After any transaction change, metadata change, exclusion change, transfer pair change, quarantine resolution, rate sync, or manual rate override, journals must be reprocessed before reports are trusted again.

Entry ids are stable across rebuilds. Unchanged entries and quantity postings
retain their `created_at`; changed entries rewrite their whole `occurred_at`
group so same-timestamp reads remain in id order. `processed_at` identifies every
rebuild, including a no-op, and advances by one microsecond if the clock has not
advanced. Explanation references expire on every rebuild, while list cursors
can survive one that leaves their rows unchanged. See the
[storage contract](r4-journal-pipeline.md#2-deterministic-entry-ids-and-changed-row-writes).

## Transfers

Cross-wallet self-transfers are auto-detected when both legs share the same
typed physical `(chain, network, txid, asset identity)` and principal. A
64-hex provider/order id is not a txid merely because it has the same shape;
the transaction graph or an explicit txid-named import field must establish
that type. Deterministic booked moves are kept out of review.
When the ownership graph proves a move but the journal declines to guess a
destination or split, `transfers suggest` surfaces an `ownership_graph` card
in the transfer tab instead of leaving the proof quarantine-only. It is exact
only when canonical scope and whole-row amounts close; source ambiguity and
amount-only destination reuse remain strong/manual.

### Self-transfer derivation

Same-`txid` matching alone misses the cases users hit — a destination wallet
that wasn't synced for the period (no inbound row), CSV imports whose `txid`
columns don't line up, and one spend fanning out to several owned wallets.
Kassiber closes these with a shared ownership pipeline during
`journals process`; its passes **supplement** same-`txid` matching and never overrule
a manual pair. A derived move carries its basis across with no disposal, and is
tagged `ownership_derived` in the transfer audit (its journal entry reads
"proven by address ownership").

1. **Address-ownership deriver** (valued on-chain graph). For an on-chain
   outbound whose full `vin`/`vout` are stored in `raw_json`, each output's script is
   classified against the profile-wide ownership index, restricted to the source
   wallet's `(chain, network)`. An output paying an address owned by another of
   your wallets is a self-transfer leg — proven from the graph, no heuristic. It
   reuses a recorded destination row on a shared `txid`, synthesizes the inbound
   leg for a true sync gap, and decomposes a 1→N fan-out. Anything it cannot
   prove safely (mixed-owner inputs, ambiguous N:M routing, a stale-RBF amount
   mismatch, an output owned by two wallets) is **left on its existing path and
   flagged for review** — never mis-booked. Bitcoin rows usually provide the
   whole graph. Liquid sync persists and merges only the non-secret input/output
   values each connected wallet can locally unblind; unknown/conflicting
   confidential value blocks the component instead of being guessed.

2. **Multi-source consolidation deriver.** A conservative N→1 Bitcoin
   consolidation is decomposed when every input is owned, the graph has exactly
   one owned destination and no external output, and amounts conserve exactly.
   The network fee is assigned once rather than copied from every source row.

3. **Recorded fan-out decomposer** (chain-agnostic fallback). When a 1→N
   self-transfer's legs were *all* synced but no readable valued graph exists —
   for example an older Liquid record or a graphless CSV import — the rows
   themselves are enough only when they share a canonical physical
   `(chain, network, txid, consensus asset identity)` across two or more of your wallets and the
   amounts conserve (`out.amount == Σ in.amount`). Provider/import labels are
   never transaction identity. A single outbound fanning to ≥2 recorded
   destinations is decomposed into per-leg moves. Multi-source consolidations
   and any group whose amounts don't fully conserve (a destination wasn't
   synced) are left to the `owned_fanout_unresolved` quarantine. The custody
   interpreter applies that hold to the whole group before arbitration, so no
   leg books as a disposal or a fresh acquisition. A clean one-outbound,
   one-inbound transfer is never held this way; its unmatched remainder stays a
   presumed external disposal.

**Scope.** Graph-based ownership derivation covers Bitcoin and sufficiently
valued local Liquid evidence. A Liquid leg that remains confidential or has
conflicting asset/value observations stays on the review path. BTC↔L-BTC pegs,
Boltz swaps, and submarine swaps are still Bitcoin movements for accounting,
but the ledger keeps `BTC` and `LBTC` separate for rail visibility; pair those
with `transfers pair` or an atomic custody component.
For multi-leg wallet migrations, consolidations, or missing historical wallets,
use the atomic [custody-component workflow](custody-components.md) instead of
forcing independent pairs.

Every carrying-value pair is also checked at the write boundary and again when
journals are rebuilt. If both legs have known Bitcoin network domains, they must
agree: mainnet, testnet, signet, and regtest basis can never be carried into one
another. This physical guard is country-neutral and also protects historical or
legacy pair rows created before the current command validation.

The pipeline is only as complete as the ownership index and recorded rows.
Kassiber keeps prior descriptor, xpub script-type, and address-list ownership
material privately inside the encrypted wallet config when a wallet migrates,
so old receive/change scripts continue to resolve. The default derivation depth
is 500 per branch and follows observed inventory floors. A wallet with deeper
history can raise it (hard cap: 20,000):

```bash
python3 -m kassiber wallets update --wallet treasury \
  --config '{"ownership_scan_to_index":1000}'
```

Higher values make journal/graph index construction more expensive. Truly
unknown address-list change or a script beyond the configured ceiling remains a
known review edge; Kassiber does not classify an unexplained residual as change
from amount alone because that would hide legitimate partial payments.

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

`transfers suggest` marks evidence exact only behind explicit gates: a
source-qualified canonical Lightning hash with compatible whole-row semantics;
a unique provider key with canonical route txids and explicit full-row integer
msat principals; or a witness-proven unique HTLC funding outpoint with amount
coverage. Provider send/receive msat amounts accept nonnegative JSON integers
or strings of ASCII decimal digits with optional surrounding whitespace.
Booleans, fractional numbers, signs, separators, superscripts and other
non-ASCII digits do not establish amount evidence. Malformed values supply no
principal claim and never abort matching or establish exactness.
Everything else stays strong/manual, except same-asset legs already
observed inside two *different* on-chain transactions: one movement between two
of your wallets is one physical transaction, so distinct txids are never paired
from time/amount proximity (use `transfers pair` if a multi-hop route really
links them). Legs sharing a txid, and legs whose transaction is unknown, stay
heuristic/manual candidates. A blocked ownership proof
(`method=ownership_graph`) is always manual-confirm-only. Ownership-graph cards carry
only wallet ids/labels, transaction ids, amounts, and stable reason codes. They
are manual-confirm-only: rules and bulk-pair operations never apply them.
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
- a reviewed same-asset link can reuse one side across multiple active pairs:
  one outbound can be manually linked to several owned inbound legs, and several
  outbound legs can be manually linked to one owned inbound leg. Kassiber books
  those reused-leg links as one grouped carrying-value movement so the total
  source quantity, destination quantity, and fee are counted once.
- reviewed `coinjoin` / `whirlpool` pairs are same-asset carrying-value links
  for manually accepted ownership hops when descriptor history is incomplete;
  they preserve basis but do not prove the full privacy graph or counterparty set
- same-asset `--policy taxable` is rejected; leave those legs unpaired if you want normal SELL + BUY treatment
- cross-asset and layer-transition pairs stay one-to-one: a transaction leg
  cannot belong to multiple active BTC↔LBTC / Lightning swap links until
  explicit multi-leg swap accounting exists
- cross-asset pairs are always stored for audit
- cross-asset `--policy carrying-value` is supported for BTC ↔ LBTC rail changes on every profile, because both assets represent the same Bitcoin exposure; Austrian books (`tax_country=at`) additionally use the engine's Austrian multi-asset hook for reviewed carrying-value swaps
- generic books default BTC ↔ LBTC suggestions, omitted-policy manual pairs, and omitted-policy asset-specific auto-pair rules to `carrying-value` while `bitcoin_rail_carrying_value` is enabled (default); turn it off with `profiles set --no-bitcoin-rail-carrying-value` when an adviser wants taxable-by-default rail changes
- explicit per-pair policy still wins: `--policy taxable` keeps SELL + BUY treatment, and `--policy carrying-value` remains available for BTC ↔ LBTC rail changes
- cross-asset `--policy taxable` keeps the normal SELL + BUY treatment
- non-Austrian books still reject `--policy carrying-value` for other cross-asset swaps
- cross-asset swaps are never auto-paired from time / amount heuristics during report generation; use `transfers pair` when those links matter for tax treatment
- swap-routed payments or receipts should stay unpaired unless both legs are known owned-wallet legs of the same user

Manual pairs override auto-detection. `transfers suggest` still suppresses
transactions that already have an active reviewed link. Use
`transfers components plan --action create` for 1:N, N:1, N:M, multi-hop
migrations, or missing intermediate wallets. Model a missing owned hop with an
`untracked_wallet`, author explicit allocations for genuine N:M, and activate
the complete component atomically after anchor coverage and conservation pass.

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

A price override, from the CLI or a review plan's `price_override`, is in
the book's currency and is stored as exact with that currency, whatever
currency an unpriced row was imported with.

Each row explains itself: `category`, `blocks_reports`, the `root` for rows
that only follow from another transaction, `evidence` and the next `actions`
(see [why a transaction is quarantined](custody-resolution.md#why-a-transaction-is-quarantined)).

Quarantine causes typically include:

- missing spot price
- missing cost basis
- insufficient lots
- ambiguous or unsupported tax semantics
- `owned_fanout_unresolved` — one canonical `(chain, network, txid, asset)`
  moves coins between owned wallets with more than one leg on either side
  (fan-out, consolidation, N:M), and no derivation pass or reviewed component
  resolved every positive leg (e.g. a destination wasn't synced, or several
  sources lack a readable graph). Every leg of the group is held. When a leg
  carries a readable graph, the address-ownership deriver remains the authority
  and flags what it cannot prove; the group is then held only when some legs
  were resolved and others were not, so an unsupported same-`txid` receipt
  cannot hold a graph-proven external payment.
- `unscoped_transfer_review` — outbound and inbound rows in different own
  wallets share a non-blank import/provider id but have no canonical Bitcoin
  `(chain, network, txid)` scope, and no pair or component covers them. Import
  labels alone cannot carry basis, so the rows are held instead of booking a
  disposal plus a fresh acquisition; connect the missing wallet evidence or
  pair/resolve the custody component explicitly.
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
- `pending_onchain_confirmation` — a chain sync payload explicitly reports the
  transaction as mempool/unconfirmed. It is held out of tax booking, and out of
  verified custody movements, until a synced leg proves confirmation; graphless
  CSV/provider rows are unaffected. The other endpoint of a candidate movement
  recorded under a different transaction (for example the funding leg of an
  HTLC route whose claim is pending) is held as
  `transfer_pair_dependency_blocked` rather than booked as a disposal.
  Esplora and Electrum payloads currently carry this explicit boolean. Bitcoin
  Core RPC wallet rows do not yet stamp it, so Core mempool activity remains on
  the prior journal path because Kassiber cannot distinguish those rows from
  graphless imports using `confirmed_at` alone.
- `conflicting_spend` — transactions share a prevout. The unconfirmed
  replacement (or every conflicting txid when the winner is ambiguous) is held
  from tax booking and cannot become a verified custody movement. Sync and
  verify which txid won; this is not a normal carrying-value pairing or
  bulk-pair action.
- Pending and conflicting holds (and the dependency holds they cause) are
  transient: they keep rows out of booking and custody moves, but they are not
  persisted as custody quantity issues, so they neither freeze later basis nor
  block report generation. Report readiness still lists them.
  Conflict detection requires stored input outpoints. Graphless CSV/provider
  rows without `vin`/prevout evidence cannot be recognized as stale RBF or
  double-spend siblings; verify or resync them from a chain-backed wallet before
  treating competing histories as settled.

The full-report XLSX `Verify!B2` banner caches the verdict of the same cached
Control and Disposals checks written to the workbook. A cached failing check
therefore displays a mismatch even in viewers that do not recalculate formulas;
the live formula remains available for later spreadsheet edits.

Exit-tax previews and PDF/XLSX estimates report the book's selected accounting
method. They reconstruct global inventory and wallet quantities from journal
entries before the next civil midnight (exclusive): Europe/Vienna for Austrian
books, UTC for generic books. This includes fractional seconds and compares
exact instants even for legacy offset timestamps. Internal transfers move wallet context without
changing global inventory; income recognition is not counted twice.
Dashboard wallet tax-free hints retain the journal's transfer-regime flows,
but are cleared per asset when the shared consumed-lot inventory has no
Altbestand left, including when another wallet's sale consumed those lots.
They do not identify physical tax lots within a globally pooled book.
FMV chooses the closest cached or transaction quote strictly before that same
cutoff. Either source must be at most 24 hours old relative to the cutoff;
exactly 24 hours is allowed. At equal instants the cache wins, retaining manual
priority over provider rows. This is the current owner-selected policy, open to
owner revision. Older, future or undated quotes cannot fill a valuation.
The source and actual quote timestamp, rendered in UTC with a `Z` suffix,
are shown in the PDF, XLSX and desktop FMV card. Missing FMV sets incomplete readiness
(`status.missingFmvAssets`): the desktop shows a top banner above Export,
affected values remain null, and PDF/XLSX handoffs are labelled incomplete
drafts. Preview and export remain available for review; missing prices prevent
a complete estimate. Wallet-context assets use the same lookup,
and the XLSX assumptions sheet discloses quarantines and incomplete estimates.
Missing FMV for any held asset, including a wallet-only rail without an
acquisition bucket, keeps market-value, gain and estimated-tax totals null;
known quantities and basis remain available for review.
Exit-tax exports remain valuation drafts outside the ordinary realized-report
snapshot system; see [saved and filed report impact](custody-components.md#saved-and-filed-report-impact).
PDF/XLSX drafts stamp the journal input version alongside the FMV quotes for
manual comparison after changes; this does not register a saved/filed snapshot.

### Exit-tax departure day and timezone

Austrian books use the Europe/Vienna civil departure day, including DST,
consistent with the Alt/Neu cutoff and realized capital-gains tax year.
Generic books use the UTC day. A date-only departure names that civil day;
a departure supplied with a time and offset is converted to the jurisdiction's
civil day first. The exclusive cutoff is the next midnight in that timezone,
converted to UTC for inventory, cached quotes and transaction quotes together.
Timed departures require an explicit offset (`Z` or, for example, `+01:00`).
A date-time without an offset is rejected with a typed `validation` error;
use a date alone to name a civil day without specifying an instant.
PDF and XLSX drafts stamp `departureTimezone` as well as the departure date.
Managed desktop and AI export filenames use that resolved civil date
(`YYYY-MM-DD`), so timed inputs cannot put offset punctuation in filenames.
The desktop takes its default departure date from the core and displays that
timezone. The calendar's today outline uses the core's default civil day,
including after a different departure date is selected.
This replaces the earlier open timezone question with the owner's decision.

For departure 2024-12-31, a receipt at 2025-01-01 00:30 Europe/Vienna
(2024-12-31 23:30Z) is excluded from an Austrian book. In summer,
2024-04-01 00:30 Europe/Vienna (2024-03-31 22:30Z) is likewise excluded
from departure 2024-03-31. The FMV age limit remains 24 elapsed hours;
it does not expand to cover a 25-hour autumn departure day.

## Rates and tax input quality

CSV exports stage ordinary files in the destination directory, preserve file
modes, flush and fsync before replacement, and fsync the directory where
supported. Symlinks are written through. Devices and pipes are written directly;
report exports to these non-regular targets succeed and explicitly report that
saved-artifact registration was skipped. No saved report snapshot is created.
Hard-linked files and existing writable files whose directory cannot hold a
staged sibling are written in place. These compatibility paths are not atomic:
an interrupted write can leave partial output, as with `open("w")`.
Storage exhaustion (`ENOSPC` or `EDQUOT`) during staging raises without opening
the existing destination for writing, so the previous complete report survives.
CSV storage failures return `export_write_failed` with only the requested
destination and OS error number, including staging failures and when the target
becomes a directory during writing; random staging filenames are not exposed.

If transactions do not already include usable fiat pricing, Kassiber first tries to fill them from the local rates cache during journal processing. When a transaction has a known `confirmed_at` timestamp, Kassiber prices from that confirmation time; otherwise it falls back to `occurred_at`. `LBTC` / `L-BTC` transactions use the BTC fiat rate because Liquid Bitcoin is pegged one-to-one with BTC.

All price write paths use shared finite decimal validation, including overrides,
history reverts, manual/cached rates, BTCPay, imports, documents and source-of-funds sources. Values must
fit finite numeric output, use exponents between -308 and 308, and have zero or
magnitude at least 1e-13 and below 1e19. The upper bound reserves the engine's
13 decimal places within its 32-digit precision; numeric mirrors must also stay
below that limit after float conversion. The lower bound prevents the engine
from comparing a positive price as zero.
Negative zero normalizes to zero. Derived totals (`rate * amount`) and unit
rates (`value / amount`) obey the same limits at write time. Invalid stored
transaction pricing is quarantined as `invalid_transaction_price`, with the
transaction and cause, while the rest of the rebuild completes. An unusable
cached valuation stores no derived price and receives the same quarantine.
These holds block dependent basis through the existing exposure-pool barriers.
Raw transaction exports leave invalid prices blank. Invalid legacy payouts remain listable and deletable;
correct the source price or delete and recreate the invalid review before
rebuilding journals. Split payout allocations quarantine their whole source as
`split_payout_requires_review` until reviewed, without pricing a residual or
aborting other transactions. Legacy payout migration records a blocking migration issue
when its stored price is invalid.

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

- Austrian books process through the tax engine's Austrian country on the same `KassiberTaxEngine` path as generic books
- Kassiber keeps provenance capture, transfer preparation, the reviewed swap pairs, and current disposal-category / Kennzahl mapping; the engine normalizes the rows and writes the swap markers
- BTC ↔ LBTC `--policy carrying-value` pairs are supported on every profile; Austrian cross-asset carrying-value pairs feed the engine's Austrian multi-asset carry path
- Austrian E 1kv export is available through `reports austrian-e1kv`,
  `reports export-austrian-e1kv-pdf`, `reports export-austrian-e1kv-xlsx`,
  and `reports export-austrian-e1kv-csv`; `reports austrian-tax-summary` and
  `reports export-austrian` are friendlier aliases for the same annual Austrian
  handoff, including outside-E-1kv rows such as KZ 801 when present
- The E 1kv's crypto Kennzahlen 172, 174 and 176 and their 27.5 % rate cover
  income and realisations from 1 March 2022, Vienna time
  (`CRYPTO_KENNZAHLEN_FROM` in `core/austrian.py`). A classified row dated
  earlier is stated in no field by Reports, the Kennzahl summary, the
  exports, the year handoff and the exit tax; KZ 801 is unaffected. Audit
  packages and the swap review show the journal's stored field as it is. The E 1kv export
  refuses a year before 2022, and a 2022 with rows from January or February,
  which followed the old rules unless the taxpayer opted otherwise. The
  indicative tax is `not_applicable` before 2022, and withheld
  (`crypto_rows_before_2022_03`) for a 2022 with such rows. Kassiber does not restate
  those rows under the old rules (the tax engine owns the classification); a
  Steuerberater does
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
