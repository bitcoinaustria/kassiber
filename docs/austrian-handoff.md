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
` \t\n,`. Free-form description can follow the markers but must not be
the protocol — typed fields on `NormalizedTaxEvent` are the source of
truth inside Kassiber; the adapter serializes them at the rp2 boundary.

## Typed source of truth on the Kassiber side

`kassiber/core/tax_events.py` defines the fields; `kassiber/core/austrian.py`
defines classification and the `AT_NEU_CUTOFF` constant; `kassiber/core/engines/rp2.py`
serializes into rp2's notes wire format in `_compose_event_notes` /
`_compose_transfer_notes` and honors `carried_basis_fiat` for incoming
swap legs.

| Field | Type | Populated by |
| --- | --- | --- |
| `at_regime` | `"alt" | "neu" | None` | Inbound rows: direct from the 2021-03-01 Europe/Vienna acquisition cutoff. Outbound rows: same cutoff by default, but post-cutoff disposals fall back to `alt` when only Alt inventory remains in scope. Future: explicit row annotations. |
| `at_pool` | `str | None` | v1: wallet_id. Future: configurable per profile. |
| `at_swap_link` | `str | None` | Engine classifier tags both legs of a Neu cross-asset pair with the pair id. |
| `carried_basis_fiat` | `Decimal | None` | Incoming leg of a Neu swap. v1: unset (quarantined). Future: Option A two-pass compute. |

## Disposal bucketing contract

rp2 Phase 9 exports `AtDisposalCategory` and `classify_disposal(gain_loss)`
from `rp2.plugin.country.at`. Kassiber consumes that API when it turns
`computed_data.gain_loss_set` into persisted journal rows:

- rp2 decides the semantic category from the matched lot, swap marker,
  and holding period.
- Kassiber persists the resulting `at_category` string on journal rows.
- Kassiber maps that semantic category onto current BMF / FinanzOnline
  Kennzahlen via its own table so tax-form wiring can evolve without
  re-implementing Austrian tax semantics.

Current Kassiber mapping:

| `AtDisposalCategory` | Kassiber `at_category` | Current Kennzahl |
| --- | --- | --- |
| `INCOME_GENERAL` | `income_general` | `172` |
| `INCOME_CAPITAL_YIELD` | `income_capital_yield` | `175` |
| `NEU_GAIN` | `neu_gain` | `174` |
| `NEU_LOSS` | `neu_loss` | `176` |
| `NEU_SWAP` | `neu_swap` | none |
| `ALT_SPEKULATION` | `alt_spekulation` | `801` |
| `ALT_TAXFREE` | `alt_taxfree` | none |

One taxable event can split across multiple gain/loss rows in rp2, so
Kassiber groups Austrian realized journal rows by `(taxable_event,
at_category)` rather than by transaction id alone. That keeps mixed Alt
holding-period cases and future mixed-income cases representable without
guessing in the report layer.

## Swap basis-carry (§ 27b Abs 3 Z 2 EStG)

For a matched crypto-to-crypto swap, rp2 zeroes the gain on the
outgoing Neu leg and depletes the pool at its running average. The
**incoming** leg's basis is Kassiber's responsibility: it must be seeded
into rp2's `InTransaction` as `fiat_in_with_fee = outgoing_amount * pool_avg_at_swap_time`
so the destination asset's pool inherits the carried basis.

### v1 scope (Option C — quarantine)

For every cross-asset pair under an AT profile:

- **Outgoing leg is Alt (acquired on/before 2021-02-28 Vienna):** the
  pair realizes. Both legs flow through as normal SELL + BUY. rp2's AT
  plugin ignores `at_swap_link` for Alt, so we deliberately do not
  emit it either — the lot-pairing audit trail reflects a real
  disposal and acquisition, not a tagged-but-ignored swap.
- **Outgoing leg is Neu:** both legs are **quarantined** with reason
  `at_swap_basis_carry_unresolved` and detail
  `{outgoing_asset, incoming_asset, out_amount, at_swap_link, reason_code: "needs_two_pass_compute"}`.
  The legs are excluded from the per-asset normalization pass, so
  no `OutTransaction` or `InTransaction` reaches rp2 for either side.
  Operators resolve the quarantine manually (override or exclusion)
  until Option A lands.

### v2 upgrade path (Option A — topological two-pass)

The planned upgrade reshapes `compute_tax` to honor a topological order
over cross-asset swaps so the outgoing asset's pool average is available
when the incoming leg is being built:

1. Pre-pass: walk all AT events across all assets in timestamp order,
   maintaining a `pool_avg_by(asset, pool_id)` running state.
2. For each cross-asset swap, look up
   `avg = pool_avg_by(out_asset, out_pool)` at the swap timestamp and
   populate `carried_basis_fiat = out_amount * avg` on the incoming
   `NormalizedTaxEvent`.
3. Feed the fully annotated events into the existing per-asset
   `normalize_tax_asset_inputs` + rp2 compute loop.

The quarantine site in `GenericRP2TaxEngine._classify_at_cross_asset_pairs`
carries a `TODO` comment pointing here; lifting the quarantine without
implementing the two-pass would silently produce zero-basis incoming
legs (wrong).

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
