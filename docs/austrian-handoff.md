# Austrian tax handoff contract (Kassiber ↔ rp2)

Kassiber is the marker emitter and Austrian reporting layer. The rp2 AT
plugin (`rp2.plugin.country.at`) is the tax-semantics interpreter and
pool-math engine. This doc pins the contract between them and records v1
scope decisions so future commits can tighten the handoff without
rediscovering them.

## Wire format

rp2 reads three markers from `InTransaction.notes` / `OutTransaction.notes`:

| Marker | Shape | Effect in rp2 |
| --- | --- | --- |
| `at_regime=alt` / `at_regime=neu` | flag | forces regime, overrides the 2021-03-01 Europe/Vienna date cutoff |
| `at_pool=<id>` | non-empty id | partitions the Neu moving-average pool; absent → `"default"`; ignored for Alt |
| `at_swap_link=<id>` | non-empty id required | Neu outgoing leg: zero-gain + pool depletes at avg. Alt: marker ignored. Empty id → rp2 raises `RP2ValueError` |

Multiple markers can coexist on the same `notes` separated by any of
` \t\n,`. rp2 parses markers as exact tokens, so unrelated free-form
text like `prefixed_at_swap_link=...` does not trigger swap handling.
Free-form description can follow the markers but must not be the
protocol — typed fields on `NormalizedTaxEvent` are the source of truth
inside Kassiber; the adapter serializes them at the rp2 boundary.

Kassiber must emit `at_swap_link` only for cross-asset `SELL` disposals
whose paired incoming leg is present. rp2 rejects empty swap ids,
duplicate/conflicting markers, same-asset swap links, orphan swap links,
and `at_swap_link` markers on non-`SELL` disposals.

## Typed source of truth on the Kassiber side

`kassiber/core/tax_events.py` defines the fields; `kassiber/core/austrian.py`
defines classification and the `AT_NEU_CUTOFF` constant; `kassiber/core/engines/rp2.py`
serializes into rp2's notes wire format in `_compose_event_notes` /
`_compose_transfer_notes`. Carried-basis computation lives in rp2's
country-level `compute_tax_for_assets` hook.

| Field | Type | Populated by |
| --- | --- | --- |
| `at_regime` | `"alt" | "neu" | None` | Inbound rows: direct from the 2021-03-01 Europe/Vienna acquisition cutoff. Outbound rows: same cutoff by default, but post-cutoff disposals fall back to `alt` when only Alt inventory remains in scope. Future: explicit row annotations. |
| `at_pool` | `str | None` | v1: wallet_id. Future: configurable per profile. |
| `at_swap_link` | `str | None` | Engine classifier tags both surviving legs of a reviewed Neu cross-asset carrying-value pair with the pair id. |

## Receipt and disposal bucketing contract

Before `compute_tax`, Kassiber maps explicit inbound `transactions.kind`
values onto rp2 transaction types. Today the adapter promotes only
unambiguous earn-like kinds:

- `staking` -> `STAKING`
- `interest`, `lending_interest` -> `INTEREST`
- `mining`, `mining_reward` -> `MINING`
- `airdrop` -> `AIRDROP`
- `hardfork`, `hard_fork` -> `HARDFORK`
- `income`, `routing_income` -> `INCOME`
- `wages` -> `WAGES`

Generic source-refresh / CSV receives such as `deposit`, `buy`, or Phoenix
transport types still go through rp2 as `BUY`. Kassiber does not invent
income semantics for unlabeled inbound rows: explicit `kind` values are
the only promotion signal in v1.

rp2 Phase 9 exports `AtDisposalCategory` and `classify_disposal(gain_loss)`
from `rp2.plugin.country.at`. Kassiber consumes that API when it turns
`computed_data.gain_loss_set` into persisted journal rows:

- rp2 decides the semantic category from the matched lot, swap marker,
  and holding period.
- Kassiber persists the resulting `at_category` string on journal rows.
- Kassiber maps that semantic category onto current BMF / FinanzOnline
  Kennzahlen via its own table so tax-form wiring can evolve without
  re-implementing Austrian tax semantics. RP2's category names and inline
  category comments are semantic hints, not the export-code source of truth.

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

`kassiber reports austrian-e1kv` is the canonical annual export. The
friendlier `reports austrian-tax-summary` and `reports export-austrian`
aliases use the same builder and data. The structured output includes
Steuerbericht-style sections 1.1-4.5, with unsupported areas rendered
as explicit zero-value placeholders instead of being silently omitted.
The XLSX handoff follows the same section set with an overview sheet,
numbered tabs, and an explanatory notes sheet. The CSV bundle mirrors that
layout as separate files because the sections do not all share one table
schema.

One taxable event can split across multiple gain/loss rows in rp2, so
Kassiber groups Austrian realized journal rows by `(taxable_event,
at_category)` rather than by transaction id alone. That keeps mixed Alt
holding-period cases and current income/disposal splits representable
without guessing in the report layer.

## Swap basis-carry (§ 27b Abs 3 Z 2 EStG)

For a matched crypto-to-crypto swap, rp2 zeroes the gain on the
outgoing Neu leg and depletes the pool at its running average. The
**incoming** leg's carried basis is rp2's responsibility: Kassiber emits
the reviewed `at_swap_link=<id>` markers, then rp2 interleaves the
affected assets through `compute_tax_for_assets` so the destination pool
inherits `outgoing_amount * source_pool_avg_at_swap_time`.

### Current scope (native rp2 multi-asset carry)

For every cross-asset pair under an AT profile:

- **`policy=taxable`:** the pair remains a normal SELL + BUY. Kassiber
  records the audit link in `cross_asset_pairs`, but does not emit
  `at_swap_link`.
- **`policy=carrying-value` + outgoing leg is Alt (acquired on/before
  2021-02-28 Vienna):** the pair still realizes. rp2's AT plugin ignores
  `at_swap_link` for Alt, so Kassiber deliberately does not emit it
  either — the lot-pairing audit trail reflects a real disposal and
  acquisition, not a tagged-but-ignored swap.
- **`policy=carrying-value` + outgoing leg is Neu:** Kassiber annotates
  both surviving legs with `at_swap_link=<pair_id>`, validates the
  cross-asset marker shape, then calls rp2's native multi-asset compute
  hook. rp2 owns the ordering and carried-basis math.

The implementation works as follows:

1. Kassiber normalizes and prepares all assets once without swap markers,
   so rows with missing pricing, missing inventory, or other readiness
   blockers are quarantined before they can create orphan markers.
2. For each reviewed Neu carrying-value pair whose two legs survived
   preparation, Kassiber emits the same non-empty `at_swap_link` on both
   legs.
3. Kassiber runs rp2's country-level `compute_tax_for_assets` hook. For
   Austrian profiles, rp2's native runner orders the affected assets,
   derives the source pool average from the moving-average engine, and
   applies the effective fiat basis override to the incoming lot.

This is direction-agnostic: both BTC->LBTC peg-ins and LBTC->BTC
peg-outs use the same handoff.

### Direct swap payouts

`transfers payouts create` covers the privacy/sale pattern where the
user sends one owned asset to a swap provider and the provider settles
the target asset directly to an external recipient or exchange. There is
no owned inbound transaction to pair, so Kassiber stores a reviewed
`direct_swap_payouts` row instead of inventing a recipient wallet.

For Austrian cross-asset `policy=carrying-value` payouts, Kassiber
synthesizes the target-asset settlement legs only inside journal
processing:

1. The real source outbound and synthetic target inbound receive the
   same `at_swap_link=direct-payout:<id>` marker.
2. rp2 carries the source pool basis onto the synthetic target
   acquisition.
3. A second synthetic target outbound immediately disposes that carried
   basis to the external recipient or exchange.

Persisted journal entries still reference the real source transaction id.
This keeps the swap itself neutral while the payout/sale remains visible
as a taxable disposal.

### Fallback quarantines

Kassiber still quarantines both legs when a carrying-value swap cannot be
fed into rp2 safely. The current reason is
`at_swap_basis_carry_unresolved`, with `reason_code` indicating the
failure mode:

- `missing_spot_price`: one or both legs lack the price data rp2 still
  needs on the raw event.
- `pricing_review_required`: imported pricing exists but needs operator
  review before it can feed tax processing.
- `unsupported_tax_direction`: one of the paired rows is not a normal
  inbound/outbound tax event.
- `swap_leg_unavailable`: a reviewed pair points at rows that normalized
  away before a more specific readiness reason was available.

Those quarantines are no longer the default Austrian swap path; they are
only the safety net when the swap cannot be annotated correctly.

## Disambiguation rule

Unmarked disposals where both Alt and Neu lots are available raise
`RP2ValueError` on the rp2 side. Kassiber is expected to resolve the
ambiguity by emitting an explicit `at_regime=` marker on the disposal.
In v1 Kassiber still defaults post-cutoff disposals toward Neu, but it
falls back to `at_regime=alt` once only Alt inventory remains. Mixed
Alt+Neu holdings are still a caller-policy problem and may require a
future `at_regime_override` raw-row column.

## Cutoff constant duplication

`AT_NEU_CUTOFF` is declared independently in:

- `rp2/plugin/country/at.py` (reader side)
- `kassiber/core/austrian.py` (writer side)

Both must point to `2021-03-01 00:00:00 Europe/Vienna`. If the Austrian
legislator ever amends the cutoff, both repos must ship a coordinated
revision — the Kassiber-side change can land first (it only affects
regime tagging), then the rp2-side change (so unmarked events classified
via the new cutoff are interpreted consistently by the reader).
