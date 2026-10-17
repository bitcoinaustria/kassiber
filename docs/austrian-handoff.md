# Austrian tax handoff contract (adapter ↔ tax engine)

Kassiber's Python core is the marker emitter, through the adapter in
`kassiber/core/engines/rp2.py`, and the Austrian reporting layer. The Austrian
country of Kassiber's [tax engine](reference/tax-engine.md)
(`tax-engine/core/src/austria.rs`) is the tax-semantics interpreter and
pool-math engine. It reproduces the RP2 AT plugin (`rp2.plugin.country.at`)
exactly, and RP2 remains the tests' parity oracle, so every rule below is
unchanged from the RP2 contract. This doc pins the contract between adapter
and engine and records v1 scope decisions so future commits can tighten the
handoff without rediscovering them.

## Wire format

The engine reads three markers from `InTransaction.notes` / `OutTransaction.notes`:

| Marker | Shape | Effect in the engine |
| --- | --- | --- |
| `at_regime=alt` / `at_regime=neu` | flag | forces regime, overrides the 2021-03-01 Europe/Vienna date cutoff |
| `at_pool=<id>` | non-empty id | partitions the Neu moving-average pool; absent → `"default"`; ignored for Alt. The adapter maps its generic `global` pool id to `"default"` |
| `at_swap_link=<id>` | non-empty id required | Neu outgoing leg: zero-gain + pool depletes at avg. Alt: marker ignored. Empty id → the engine raises `EngineValueError` |

Multiple markers can coexist on the same `notes` separated by any of
` \t\n,`. The engine parses markers as exact tokens, so unrelated free-form
text like `prefixed_at_swap_link=...` does not trigger swap handling.
Free-form description can follow the markers but must not be the
protocol — typed fields on `NormalizedTaxEvent` are the source of truth
inside Kassiber; the adapter serializes them at the engine boundary.

Kassiber must emit `at_swap_link` only for cross-asset `SELL` disposals
whose paired incoming leg is present. The engine rejects empty swap ids,
duplicate/conflicting markers, same-asset swap links, orphan swap links,
and `at_swap_link` markers on non-`SELL` disposals.

## Typed source of truth on the Kassiber side

`kassiber/core/tax_events.py` defines the fields; `kassiber/core/austrian.py`
defines classification and the `AT_NEU_CUTOFF` constant; `kassiber/core/engines/rp2.py`
serializes into the engine's notes wire format in `_compose_event_notes` /
`_compose_transfer_notes`. Carried-basis computation lives in the engine's
country-level `compute_tax_for_assets` hook.

| Field | Type | Populated by |
| --- | --- | --- |
| `at_regime` | `"alt" | "neu" | None` | Inbound rows: direct from the 2021-03-01 Europe/Vienna acquisition cutoff. Outbound rows: same cutoff by default, but post-cutoff disposals fall back to `alt` when only Alt inventory remains available in the disposing wallet. Same-asset internal transfers move regime availability between wallets before later disposals are classified. Future: explicit row annotations. |
| `cost_basis_pool_id` | `str | None` | Country-neutral opaque event pool id. Every currently enabled country policy allows only `global`; the adapter serializes that id as `at_pool=default` for Austrian rows and emits no Austrian marker for generic profiles. |
| `from_cost_basis_pool_id` / `to_cost_basis_pool_id` | `str | None` | Country-neutral source and destination facts on an internal transfer. Austrian transfers must currently resolve to the same global pool. The engine has no reviewed two-ended Austrian transfer marker contract, so differing ids fail closed instead of being approximated. |
| `at_regime_basis` | `"wahlrecht" | None` | Audit-trail provenance of an outbound row's `at_regime`, serialized into notes as `at_regime_basis=wahlrecht` but **not read by the engine**. `wahlrecht` means the disposing wallet held both Alt and Neu inventory, so Neu-first was Kassiber exercising the taxpayer's KryptowährungsVO designation right on their behalf — the statutory presumption absent a designation is earliest-acquired-first. `None` means the regime was forced by the wallet's holdings (pure Alt / pure Neu) or set by an explicit `at_regime_override`. A configurable ordering (earliest-first as the legal default) is a tracked follow-up. |
| `at_swap_link` | `str | None` | Engine classifier tags both surviving legs of a reviewed Neu cross-asset carrying-value pair with the pair id. |

## Receipt and disposal bucketing contract

Before `compute_tax`, Kassiber maps explicit inbound `transactions.kind`
values onto engine transaction types. Today the adapter promotes only
unambiguous earn-like kinds:

- `staking` -> `STAKING`
- `interest`, `lending_interest` -> `INTEREST`
- `mining`, `mining_reward` -> `MINING`
- `airdrop`, `hardfork`, `hard_fork`: blocked by `acquisition_valuation_unsupported`
  for Austrian profiles; the generic-country adapter still maps these to
  `AIRDROP` / `HARDFORK`.
- `income`, `routing_income` -> `INCOME`
- `wages` -> `BUY` (raw `kind=wages` remains provenance)

Austrian zero-cost acquisitions (including qualifying airdrops, hardforks and
validation staking) are not supported by the current adapter/report section 3.2.
A label does not prove eligibility. Austrian airdrop/hardfork rows are quarantined
before the engine, including imported classifications and metadata overrides; later
same-asset disposals fail closed while that basis remains uncertain. Existing
processed books containing these rows are marked stale once on upgrade. Saved
historical outputs remain unchanged.

`STAKING` / `INTEREST` retain the meaning the Austrian country carries over
from RP2: taxable lending-style returns. Do not declare validation staking as this supported
lending type: an explicit `valuation_mode=zero_cost` acquisition review request
fails with `acquisition_valuation_unsupported`. `wages` remains a basis acquisition
without employment-income reporting. This distinction follows the
[BMF cryptocurrency treatment](https://www.bmf.gv.at/themen/steuern/sparen-veranlagen/steuerliche-behandlung-von-kryptowaehrungen.html)
and the RP2 AT plugin contract the engine reproduces; the engine adds no
Austrian computation beyond that contract.

Generic source-refresh / CSV receives such as `deposit`, `buy`, or Phoenix
transport types still go to the engine as `BUY`. Kassiber does not invent
income semantics for unlabeled inbound rows: explicit `kind` values are
the only promotion signal in v1.

The engine's Austrian country provides `AtDisposalCategory` and
`classify_disposal(gain_loss)`, which `kassiber.core.engines.native` exposes
with RP2's names and values. Kassiber consumes that API when it turns
`computed_data.gain_loss_set` into persisted journal rows:

- The engine decides the semantic category from the matched lot, swap marker,
  and holding period.
- Kassiber persists the resulting `at_category` string on journal rows.
- Kassiber maps that semantic category onto current BMF / FinanzOnline
  Kennzahlen via its own table so tax-form wiring can evolve without
  re-implementing Austrian tax semantics. The category names are semantic
  hints, not the export-code source of truth.

Current Kassiber mapping:

| `AtDisposalCategory` | Kassiber `at_category` | Current Kennzahl |
| --- | --- | --- |
| `INCOME_GENERAL` | `income_general` | `172` |
| `INCOME_CAPITAL_YIELD` | `income_capital_yield` | `172` |
| `NEU_GAIN` | `neu_gain` | `174` |
| `NEU_LOSS` | `neu_loss` | `176` |
| `NEU_SWAP` | `neu_swap` | none |
| `ALT_SPEKULATION` | `alt_spekulation` | `801` |
| `ALT_TAXFREE` | `alt_taxfree` | none |

Kennzahlen 172, 174, and 176 target the current ausländisch / self-custody
slice of E 1kv. Kennzahl 801 is old-stock speculation income for E 1 and is
carried in the same Austrian handoff as an outside-E-1kv row, not as an E 1kv
field. Kassiber does not yet persist structured domestic-provider withheld-KESt
metadata, so it cannot populate domestic-provider Kennzahlen such as 171, 173,
or 175. CLI/PDF exports must surface that assumption until the data model can
represent withheld tax.

`wages` books as an ordinary acquisition. Kassiber treats the reviewed payroll
EUR value as acquisition basis, keeps the raw transaction kind and attachments
as provenance, and leaves wage-tax reporting outside the product. It does not
create a separate employment-income journal or E 1kv row.

`kassiber reports austrian-e1kv` is the canonical annual export. The
friendlier `reports austrian-tax-summary` and `reports export-austrian`
aliases use the same builder and data. The structured output includes
Steuerbericht-style sections 1.1-4.5, with unsupported areas rendered
as explicit zero-value placeholders instead of being silently omitted.
The XLSX handoff follows the same section set with an overview sheet,
numbered tabs, and an explanatory notes sheet. The CSV bundle mirrors that
layout as separate files because the sections do not all share one table
schema.

One taxable event can split across multiple gain/loss rows in the engine, so
Kassiber groups Austrian realized journal rows by `(taxable_event,
at_category)` rather than by transaction id alone. That keeps mixed Alt
holding-period cases and current income/disposal splits representable
without guessing in the report layer.

## Swap basis-carry (§ 27b Abs 3 Z 2 EStG)

For a matched crypto-to-crypto swap, the engine zeroes the gain on the
outgoing Neu leg and depletes the pool at its running average. The
**incoming** leg's carried basis is the engine's responsibility: Kassiber emits
the reviewed `at_swap_link=<id>` markers, then the engine interleaves the
affected assets through `compute_tax_for_assets` so the destination pool
inherits `outgoing_amount * source_pool_avg_at_swap_time`.

### Current scope (engine multi-asset carry)

For every cross-asset pair under an AT profile:

- **`policy=taxable`:** the pair remains a normal SELL + BUY. Kassiber
  records the audit link in `cross_asset_pairs`, but does not emit
  `at_swap_link`.
- **`policy=carrying-value` + outgoing leg is Alt (acquired on/before
  2021-02-28 Vienna):** the pair still realizes. The engine ignores
  `at_swap_link` for Alt, so Kassiber deliberately does not emit it
  either — the lot-pairing audit trail reflects a real disposal and
  acquisition, not a tagged-but-ignored swap.
- **`policy=carrying-value` + outgoing leg is Neu:** Kassiber annotates
  both surviving legs with `at_swap_link=<pair_id>`, validates the
  cross-asset marker shape, then calls the engine's multi-asset compute
  hook. The engine owns the ordering and carried-basis math.

The implementation works as follows:

1. Kassiber normalizes and prepares all assets once without swap markers,
   so rows with missing pricing, missing inventory, or other readiness
   blockers are quarantined before they can create orphan markers. The
   marker-selection step then consults those phase-1 quarantine reasons: if
   either leg of a carrying-value pair was blocked (e.g. `insufficient_lots`
   on an over-sold outgoing leg), Kassiber quarantines the whole pair and
   skips promotion rather than marking only the surviving leg. Marking it
   would both orphan the `at_swap_link` (tripping the cross-asset validator)
   and — because at_swap rows are exempt from the single-asset quantity gate
   so the Austrian runner can resolve cross-asset basis — carry the shortfall
   into `compute_tax`, where the engine's per-account balance check aborts the
   entire multi-asset report with an uncatchable "balance went negative".
2. For each reviewed Neu carrying-value pair whose two legs survived
   preparation, Kassiber emits the same non-empty `at_swap_link` on both
   legs.
3. Kassiber runs the engine's country-level `compute_tax_for_assets` hook. For
   Austrian profiles, the engine's Austrian runner orders the affected assets,
   derives the source pool average from the moving average, and
   applies the effective fiat basis override to the incoming lot.

This is direction-agnostic: both BTC->LBTC peg-ins and LBTC->BTC
peg-outs use the same handoff.

### Direct swap payouts

`transfers payouts create` covers the privacy/sale pattern where the
user sends one owned asset to a swap provider and the provider settles
the target asset directly to an external recipient or exchange. There is
no owned inbound transaction to pair, so Kassiber stores a reviewed
`direct_swap_payouts` row instead of inventing a recipient wallet.
For split source transactions, the row can store an `out_amount` that covers
only the payout portion of the source spend; journal processing derives the
remaining same-asset self-transfer from the imported owned inbound leg.
The model is country-neutral: the reviewed `payout_fiat_value`, when
present, becomes the taxable source-row proceeds for ordinary direct
payout reviews. Cross-asset carrying-value treatment is available for
Austrian profiles and for BTC/LBTC Bitcoin-rail moves in generic profiles.

For cross-asset `policy=carrying-value` payouts, Kassiber
synthesizes the target-asset settlement legs only inside journal
processing:

1. The real source outbound and synthetic target inbound receive the
   same `at_swap_link=direct-payout:<id>` marker.
2. The engine carries the source pool basis onto the synthetic target
   acquisition.
3. A second synthetic target outbound immediately disposes that carried
   basis to the external recipient or exchange.

Persisted journal entries still reference the real source transaction id.
This keeps the swap itself neutral while the payout/sale remains visible
as a taxable disposal.

### Fallback quarantines

Kassiber still quarantines both legs when a carrying-value swap cannot be
fed into the engine safely. The current reason is
`at_swap_basis_carry_unresolved`, with `reason_code` indicating the
failure mode:

- `missing_spot_price`: one or both legs lack the price data the engine still
  needs on the raw event.
- `pricing_review_required`: imported pricing exists but needs operator
  review before it can feed tax processing.
- `unsupported_tax_direction`: one of the paired rows is not a normal
  inbound/outbound tax event.
- `swap_leg_unavailable`: a reviewed pair points at rows that normalized
  away before a more specific readiness reason was available.
- Any pre-flight ledger-gate reason (`insufficient_lots`,
  `missing_cost_basis`, `basis_provenance_incomplete`,
  `unclassified_income_kind`): a leg survived normalization but was blocked
  by the phase-1 gate, so the pair is quarantined as a unit instead of being
  promoted to a swap marker that would carry the block into `compute_tax`.

Those quarantines are no longer the default Austrian swap path; they are
only the safety net when the swap cannot be annotated correctly.

## Disambiguation rule

Unmarked disposals where both Alt and Neu lots are available raise
`EngineValueError` in the engine. Kassiber is expected to resolve the
ambiguity by emitting an explicit `at_regime=` marker on the disposal.
In v1 Kassiber still defaults post-cutoff disposals toward Neu, but it
falls back to `at_regime=alt` once only Alt inventory remains. Mixed
Alt+Neu holdings are still a caller-policy problem and may require a
future `at_regime_override` raw-row column.

## Cutoff constant duplication

The Neu cutoff is declared independently in:

- `tax-engine/core/src/austria.rs` as `NEU_CUTOFF_US` (reader side)
- `kassiber/core/austrian.py` as `AT_NEU_CUTOFF` (writer side)

Both must point to `2021-03-01 00:00:00 Europe/Vienna`. If the Austrian
legislator ever amends the cutoff, change both in one reviewed change, so
unmarked events classified via the new cutoff are interpreted consistently by
the reader. RP2 keeps the old cutoff, so the parity comparison must list the
change as a reviewed difference.
