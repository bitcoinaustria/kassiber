# Austrian Tax Support On RP2 — Design

**Status:** Austrian tax processing is active again through the shared RP2 adapter in `kassiber/core/engines/rp2.py`, using `rp2.plugin.country.at.AT` from the Kassiber-maintained fork at `https://github.com/bitcoinaustria/rp2`. This document captures the current architecture plus the remaining Austrian backlog, especially E 1kv export and review UX. See `docs/austrian-handoff.md` for the current marker / carry-basis contract.
**Module:** RP2 fork plugins for Austrian country / accounting / reports, integrated from Kassiber through `kassiber/core/engines/rp2.py`
**Report:** Planned E 1kv export layered on top of the shared journal/report pipeline and backed by RP2-fork output.
**Legal gate:** Output requires Steuerberater review before filing. A disclaimer surfaces on first use and on every report.

## Why RP2 should be the tax engine

Kassiber should be the local-first Bitcoin accounting product on top of the tax engine, not a second tax engine beside it. RP2 already has a country/accounting/report plugin architecture, and Kassiber now has a maintained fork at `bitcoinaustria/rp2`, so the strategic direction is:

- **RP2 owns tax computation.** Country semantics, accounting methods, lot math, gains/losses, and tax-report generators belong in RP2 or the Kassiber-maintained RP2 fork.
- **Kassiber owns facts and workflow.** Wallet sync, Bitcoin-node integration, import adapters, rates, provenance capture, transfer pairing, AI tagging, review/quarantine UX, and desktop/CLI presentation belong in Kassiber.
- **Kassiber prepares and explains; RP2 computes.** Kassiber normalizes raw transaction history into tax-ready facts, then feeds those facts to RP2 and renders the results back into local-first reports and UI flows.

Austria still requires capabilities that stock RP2 does not yet model cleanly:

1. **Cost basis from 2023-01-01 requires gleitender Durchschnittspreis** (moving average). Stock RP2 has no moving-average engine — all its accounting methods are lot-tracking — so the Austrian path lives on the Kassiber-maintained fork.
2. **Crypto-to-crypto swaps are non-taxable for Neuvermögen** under §27b Abs 3 Z 2 EStG, with basis carrying to the new asset. The Kassiber-maintained RP2 fork now supports this through `at_swap_link`, but Kassiber still has to pair the legs and seed the incoming carried basis correctly.
3. **Regime classification is by acquisition date, not holding period.** Coins acquired on/before 2021-02-28 are Altvermögen (old regime); after are Neuvermögen (new regime). RP2's `long_term_days` is a days-threshold, not a calendar cutoff.

Additionally, the Altvermögen rules themselves involve a 1-year Spekulationsfrist that resembles RP2's long-term threshold but only applies to the Altvermögen tranche and expires via swap, which adds state RP2 doesn't track.

The change in direction is that these gaps should be closed in the RP2 fork via Austrian plugins and any missing tax primitives, rather than by making Kassiber own Austrian tax math itself.

## Prerequisite: normalization and provenance layer

Before Austrian RP2 integration can be trusted, Kassiber needs a tax-input normalization seam between raw transactions and tax-engine logic.

Why:

- Raw `transactions` today do not reliably distinguish buy vs mining vs routing income vs inheritance vs swap.
- Manual transfer pairs and cross-asset pair audit state already affect tax behavior and need to be explicit inputs.
- Some Austrian questions require facts Kassiber may not currently observe; those cases must be quarantined or explicitly annotated rather than guessed.

Phase 0.5 therefore introduces `kassiber/core/tax_events.py`:

- Input: raw transactions, wallet metadata, transfer-pair state, explicit tax annotations, and rate lookup helpers.
- Output: typed `NormalizedTaxEvent` records for the engine.
- Rule: if the normalizer cannot prove the event type with acceptable confidence, it emits an ambiguous event and the engine quarantines it.

This layer is shared by both the generic and Austrian RP2-backed paths, so the Austrian work improves the generic engine boundary instead of forking the ingestion story.

## Legal framework (sources)

Primary:
- [BMF — Steuerliche Behandlung von Kryptowährungen](https://www.bmf.gv.at/themen/steuern/sparen-veranlagen/steuerliche-behandlung-von-kryptowaehrungen.html)
- [§ 27a EStG — jusline](https://www.jusline.at/gesetz/estg/paragraf/27a)
- [§ 27b EStG — jusline](https://www.jusline.at/gesetz/estg/paragraf/27b)
- [KryptowährungsVO § 2 — gesetzefinden.at](https://gesetzefinden.at/bundesrecht/verordnungen/kryptowahrungsvo/para-2)
- [E 1kv 2024 form PDF — BMF](https://formulare.bmf.gv.at/service/formulare/inter-Steuern/pdfs/2024/E1kv.pdf)

Secondary (cited throughout `07-austrian-tax-open-questions.md`):
- ICON Wirtschaftstreuhand
- TPA
- KPMG
- Blockpit (filing guide)
- WKO
- crypto-tax.at (practitioner guide with citations)

## Regime taxonomy — BTC-focused

| Event | Altvermögen (acq ≤ 2021-02-28) | Neuvermögen (acq > 2021-02-28) |
|---|---|---|
| BTC → EUR | 1-year Spekulationsfrist: tax-free if held >1y; else progressive up to 55% | 27.5% KESt on realized gain |
| BTC → BTC self-transfer between own wallets | Not a disposal | Not a disposal |
| BTC → altcoin swap (non-BTC crypto) | **Breaks Altvermögen**: new asset becomes Neuvermögen. Swap itself non-taxable if done under Neuvermögen rules post-reform. | Non-taxable; basis carries to new asset |
| BTC → goods/services | Disposal at FMV; 1-year rule applies | Disposal at FMV; 27.5% KESt |
| Mining (private) at receipt | Receipt becomes Neuvermögen if received post-2022-03-01; FMV income, 27.5% | FMV income, 27.5% |
| Lightning routing fees earned | Default: laufende Einkünfte at FMV, 27.5%. See AT-001 in 07. | Same. See AT-001. |
| Lending interest paid in BTC | Laufende Einkünfte at FMV on receipt, 27.5% (public placement; private placement = progressive) | Same |
| Airdrops, hardforks, bounties, staking | **Zero basis** on receipt; 27.5% on later disposal | Same |
| Gift (donor) | No income-tax event; Schenkungsmeldung § 121a BAO above threshold | Same |
| Gift (recipient) | Basis and acquisition date **carry over** (Buchwertfortführung) — Altvermögen status survives | Basis and date carry over |
| Inheritance | Buchwertfortführung per practitioner consensus. See AT-004. | Same |

For BTC-only kassiber, the altcoin-swap row is structurally interesting (it's how Altvermögen loses status) but practically rare; most users will have BTC-only flows where Altvermögen persists indefinitely.

## Cost basis rules

### Altvermögen

Always **FIFO within Altvermögen tranche, per normalized tax container**. This was the pre-reform practice and continues for Altvermögen because the Spekulationsfrist needs a per-lot holding-start date. Swapping Altvermögen to any other crypto **terminates** its Altvermögen status for that lot; the resulting asset is Neuvermögen.

### Neuvermögen pre-2023

**FIFO per normalized tax container**. This is the period between 2022-03-01 (new regime in force) and 2022-12-31 (KryptowährungsVO not yet in force). Taxpayers could prove specific allocations, but FIFO is the default and the engine's output.

### Neuvermögen from 2023-01-01

**Gleitender Durchschnittspreis (weighted moving average) per normalized tax container.**

Legal note: the statute and practitioner literature speak in terms of wallet-address-level tracking. Kassiber's MVP engine works on a **normalized tax container** abstraction so the engine interface stays stable while provenance improves over time.

- For currently supported sync/import paths, the default container is the Kassiber `wallet_id`.
- If a future backend or importer can provide stricter address/UTXO provenance, that narrower container can be used without changing the engine contract.
- If provenance is insufficient to support a legally defensible container for a given event, the event is quarantined instead of force-fit into the moving-average pool.

Algorithm per tax container per asset (BTC):

```
state: qty = 0, avg_price = 0

on acquisition of Δqty at price p_new:
    new_total = qty * avg_price + Δqty * p_new
    qty += Δqty
    avg_price = new_total / qty   (if qty > 0)

on disposal of Δqty:
    proceeds = Δqty * sale_price
    cost = Δqty * avg_price
    gain = proceeds - cost
    qty -= Δqty
    # avg_price unchanged (disposal doesn't reset the average)

on self-transfer (intra-wallet outgoing, same-owner incoming):
    no-op for the average — it's basis-carrying, not a disposal, and
    the incoming side inherits the outgoing wallet's running average
    proportional to the quantity transferred.
```

The last rule is important and subtle: moving BTC from container A to container B doesn't change A's running average for its remaining balance, but **the newly arrived qty in B is added to B's average at A's current avg_price** (not at market price, because it's not an acquisition event).

Consolidation sweeps (many-to-one) are out of published BMF guidance — see AT-003. Default behavior: weighted-average the sending wallets' averages by qty, use that as the incoming cost in the destination wallet.

### Fee treatment

- Purchase fees (on-chain or exchange): **add to cost basis** of the acquired coins
- Sale fees: **reduce proceeds** (not a separate deduction)
- Crypto-to-crypto swap fees: **ignored** (swap itself non-taxable for Neuvermögen; basis carries)
- Mining/electricity/hardware costs: **not deductible** at the 27.5% rate. Only deductible if user elects Regelbesteuerungsoption (progressive rate) — out of MVP scope; engine emits income with cost = 0 and leaves deductions to user's own E 1 filing

## Classification in normalized tax events

Each lot-origin event (acquisition, mining receipt, airdrop, income from lending) is classified by the normalizer before it reaches the engine:

```
classify(event) -> 'altvermoegen' | 'neuvermoegen'

if event.kind in ('buy', 'receive_from_external', 'receive_from_mining'):
    if event.timestamp <= 2021-02-28T23:59:59 (Europe/Vienna):
        return 'altvermoegen'
    else:
        return 'neuvermoegen'

if event.kind in ('receive_from_airdrop', 'receive_from_hardfork', 'receive_from_staking'):
    # zero-basis; always Neuvermögen
    return 'neuvermoegen'

if event.kind == 'receive_from_swap' (altcoin -> BTC, or BTC -> altcoin received side):
    # swap under Neuvermögen inherits source status if source was Neuvermögen;
    # if source was Altvermögen, the swap breaks its status and the new side is Neuvermögen
    return source_status_or_neuvermoegen_if_altvermoegen_was_source
```

Classification is part of the normalized tax-event stream for a given journal run. It is **not** written back onto raw `transactions`.

## Data model additions

### Migration `004_austrian_tax_engine.sql` (after attachments migration)

```sql
-- Explicit tax semantics for transactions whose meaning is not recoverable
-- from the raw on-chain/imported shape alone.
CREATE TABLE transaction_tax_annotations (
    transaction_id    TEXT PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
    event_type        TEXT,  -- buy / sell / spend / mining_income / routing_income / inherited_receive / gift_receive / swap_receive / ...
    provenance_json   TEXT NOT NULL DEFAULT '{}',
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
```

Amounts stored in integer **eurocents** to avoid float. The 27.5% rate is modeled as a rational: `cents * 275 / 1000`, with rounding per BMF convention (commercial rounding, half-up).

Deliberate non-decisions for MVP:

- **No `transactions.at_regime` column.** Raw transactions stay source-of-truth.
- **No `at_journal_cache` table.** Austrian processing writes through the same rebuildable journal path as the generic engine. If caching is needed later, add a disposable report cache keyed by `policy_hash`, not a second authoritative ledger.
- **No Austrian-only wallet provenance column inside Kassiber until the RP2-backed path needs it.** Add persisted Austrian metadata only if the future RP2 integration proves it is necessary.

## Engine interface

Today the shared engine seam is intentionally narrow:

```python
# kassiber/core/engines/base.py
@dataclass(frozen=True)
class TaxEngineLedgerInputs:
    rows: Sequence[Mapping[str, Any]]
    wallet_refs_by_id: Mapping[str, Mapping[str, Any]]
    manual_pair_records: Sequence[Mapping[str, Any]]


@dataclass(frozen=True)
class TaxEngineLedgerResult:
    entries: list[dict[str, Any]]
    quarantines: list[dict[str, Any]]
    intra_audit: list[dict[str, Any]]
    cross_asset_pairs: list[dict[str, Any]]
    account_holdings: dict[tuple[Any, ...], dict[str, Any]]
    wallet_holdings: dict[tuple[Any, ...], dict[str, Any]]


class TaxEngine(Protocol):
    def build_ledger_state(self, inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult: ...
```

The Austrian RP2-backed path should keep fitting this same boundary. Austrian-specific summaries such as E 1kv totals belong in a reporting layer built from the shared journal state, not in a separate Kassiber-only tax engine return type.

## RP2 fork requirements

The important implementation point is not a Kassiber-side Austrian algorithm anymore. It is the capability boundary the RP2 fork needs to provide while Kassiber keeps normalization, provenance, and orchestration outside the tax math.

Kassiber should continue to do these parts:

- start from the shared per-profile journal inputs
- reuse the same transfer detection and manual-pair application story as the generic path
- normalize raw rows into typed tax events via `kassiber/core/tax_events.py`
- quarantine events whose Austrian tax semantics cannot be derived with acceptable confidence
- persist shared ledger output and render Austrian summaries such as E 1kv on top

The RP2 fork should own these Austrian-specific semantics:

- Altvermögen classification and holding-period handling
- Neuvermögen FIFO behavior before 2023
- Neuvermögen moving-average behavior from 2023 onward
- carry-forward of basis and regime across self-transfers
- treatment of income-like receipts such as mining, routing fees, and airdrops
- Austrian loss-offset behavior within the supported regime

If a future design sketch needs pseudocode, it should live in the RP2 fork or in a fork-specific design note, not in Kassiber planning docs in a way that implies a growing Kassiber-side Austrian engine.

## E 1kv report

### PDF

- Landscape A4, BMF-aligned section ordering
- Header: profile name, tax year, fiat currency (EUR), generated timestamp, kassiber version
- Section 1: "Einkünfte aus Kryptowährungen (§ 27b EStG)"
  - 1a: laufende Einkünfte (Kennzahl **171** inländisch, **172** ausländisch)
  - 1b: realisierte Wertsteigerungen Neuvermögen (Kennzahl **173/174/175/176** per inländisch/ausländisch split)
  - 1c: Altvermögen disposals within 1-year Spekulationsfrist (no Kennzahl on E 1kv — these go to E 1 under Spekulationsgeschäfte; engine surfaces them as a separate table)
  - 1d: Altvermögen disposals with holding >1y (tax-free — surfaced as informational only)
- Section 2: Wallet-by-wallet breakdown (moving-average reconstructions per wallet per year from 2023+)
- Section 3: Journal entries list (date, kind, wallet, qty, price, cost, gain/loss, regime, note)
- Section 4: Quarantined transactions (if any)
- Section 5: Disclaimers (Steuerberater review gate, kassiber not tax advice, open questions noted from `07-austrian-tax-open-questions.md`)

### CSV

Single CSV with one row per journal entry:

```
date,tx_id,wallet,kind,at_regime,qty_msat,price_eur_cents,cost_basis_eur_cents,proceeds_eur_cents,gain_loss_eur_cents,income_eur_cents,holding_period_days,kennzahl,note
```

`kennzahl` is populated when the entry maps to an E 1kv code (171/172/173/174/175/176), null otherwise.

### Rendering tech

PDF: `reportlab` (already a candidate in kassiber; no new heavy dep). Tables, headers, embedded logo, footer with page number + generated timestamp.

CSV: stdlib `csv.writer`, UTF-8, BOM on first byte for Excel compatibility (Austrian users often open in Excel).

## Disclaimers

First use of `at` policy shows a one-time modal:

> Kassiber's Austrian tax support path is a self-help tool. It does not constitute tax advice.
>
> The output is designed to support your preparation of the E 1kv form but must be reviewed by a Steuerberater before filing.
>
> Kassiber maintains a list of genuinely unsettled questions in the Austrian crypto tax landscape (see `docs/plan/07-austrian-tax-open-questions.md`). Those notes are planning input for the current RP2-backed Austrian path; unresolved cases should surface as review notes, warnings, or quarantines rather than silent guesses.

A footer on every E 1kv PDF repeats the Steuerberater-review gate and lists any open-question defaults used in that report.

## Current policy registration

```python
# kassiber/tax_policy.py
def build_austrian_policy(profile):
    country = _load_rp2_austrian_country()
    return TaxPolicy(
        tax_country="at",
        fiat_currency=country.currency_iso_code.upper(),
        long_term_days=country.get_long_term_capital_gain_period(),
        accounting_methods=tuple(sorted(country.get_accounting_methods())),
        report_generators=tuple(sorted(country.get_report_generators())),
        default_accounting_method=country.get_default_accounting_method(),
        generation_language=country.get_default_generation_language(),
    )


# kassiber/core/engines/__init__.py
def build_tax_engine(profile):
    return GenericRP2TaxEngine(profile)
```

The current state is one shared RP2-backed adapter in Kassiber. Austrian profile selection still happens from Kassiber profiles, but the adapter chooses Austrian country, accounting, and report plugins in the RP2 fork rather than a Kassiber-side Austrian ledger builder.

## Testing

Current coverage in `tests/test_review_regressions.py`, `tests/test_austrian_classification.py`, and the Austrian snapshot fixtures exercises the live RP2-backed path and should keep growing into the broader parity suite:

- Austrian profiles process successfully through rp2's `AT` plugin
- disposal categories and Kennzahl mappings round-trip into persisted journal/report rows
- Neu cross-asset `--policy carrying-value` pairs carry basis when data is sufficient and quarantine when it is not
- income-like Austrian receipts such as staking produce both acquisition and income entries
- the shared normalization seam still quarantines unresolved Austrian inputs instead of guessing

The scenarios below remain the desired target suite as provenance support expands:

### Scenario 1: Pure Altvermögen disposal after >1 year
- Buy 1 BTC on 2020-06-01
- Sell 1 BTC on 2024-06-01 for EUR 50,000
- Expect: Altvermögen entry, holding >365 days, gain = 0 (tax-free note), no Kennzahl

### Scenario 2: Pure Altvermögen disposal within <1 year (edge: reform doesn't affect this rule)
- Buy 1 BTC on 2020-06-01 with no explicit wallet override; on-chain date alone makes it Altvermögen
- Sell 1 BTC on 2020-12-01 for EUR 30,000
- Expect: Altvermögen entry, holding <365 days, gain surfaced for E 1 Spekulationsgeschäfte — not E 1kv

### Scenario 3: Neuvermögen pre-2023 FIFO
- Buy 1 BTC on 2022-04-01 for 35,000
- Buy 1 BTC on 2022-08-01 for 20,000
- Sell 1.5 BTC on 2022-11-01 for 25,000/BTC
- Expect: FIFO consumes 1 BTC from first lot (cost 35,000, proceeds 25,000, loss 10,000), 0.5 BTC from second lot (cost 10,000, proceeds 12,500, gain 2,500), net loss 7,500 in Neuvermögen bucket

### Scenario 4: Neuvermögen from-2023 moving average
- Wallet X in 2023:
  - Buy 1 BTC on 2023-02-01 for 22,000
  - Buy 1 BTC on 2023-06-01 for 28,000  → running avg 25,000
  - Sell 0.5 BTC on 2023-09-01 for 27,000 → proceeds 13,500, cost 12,500, gain 1,000 → running state 1.5 BTC @ 25,000
- Expect: disposal entry with avg-based cost, running state preserved

### Scenario 5: Self-transfer preserves regime, carries avg
- Wallet A: 1 BTC @ avg 20,000 (Neuvermögen from-2023)
- Self-transfer 0.5 BTC to Wallet B on 2023-07-01
- Sell 0.5 BTC from Wallet B on 2023-10-01 for 25,000
- Expect: disposal entry cost = 10,000 (0.5 * 20,000), proceeds 12,500, gain 2,500

### Scenario 6: Mining receipt is income
- Receive 0.01 BTC from mining on 2023-04-01, FMV EUR 28,000/BTC
- Expect: income entry for 280 EUR; lot added to wallet state at cost 280 (for future disposal)
- Later sale at 30,000/BTC realizes 20 EUR gain (moving average recalculation)

### Scenario 7: LN routing fee (AT-001 default behavior)
- Receive 1000 sats from Lightning routing on 2023-05-01, FMV 0.28 EUR
- Expect: income entry, 0.28 EUR, with note "AT-001 default treatment — confirm with Steuerberater"
- Entry appears in PDF disclaimer section's list of AT-00x defaults applied

### Scenario 8: Altvermögen swap breakage (BTC-only edge: hypothetical, no real swap path yet)
- Buy 1 BTC on 2020-06-01
- Swap 1 BTC → 30 LTC on 2023-04-01 (imaginary altcoin support)
- Expect: Altvermögen status of 1 BTC terminates; 30 LTC starts as Neuvermögen
  - Note: kassiber is BTC-only; this scenario exists in test coverage as defensive design for potential future altcoin support, but isn't user-reachable until altcoins are added

### Scenario 9: Loss offset within Neuvermögen
- Two Neuvermögen disposals, one gain 1000 EUR, one loss 400 EUR, same tax year
- Expect: summary gain_neuvermoegen = 1000, loss_neuvermoegen = 400, kest_due = (1000-400) * 0.275 = 165

### Scenario 10: Quarantine on missing price
- Transaction on a date where `rate_lookup` returns None
- Expect: tx in quarantined list with reason; entry still recorded with price=null; summary includes quarantine count

### Scenario 11: Quarantine on ambiguous semantics
- Imported inbound BTC row with no reliable indication whether it is a gift, mining income, or a normal external receive
- Expect: normalizer marks event ambiguous, engine quarantines it, and no silent income/disposal classification is invented

## Implementation order within Phase 0.5

1. Keep `kassiber/core/tax_events.py` as the shared normalization seam and extend it only where Austrian provenance needs more explicit annotation
2. Keep `kassiber/core/engines/base.py` limited to `TaxEngineLedgerInputs` / `TaxEngineLedgerResult` and `build_ledger_state(...)`
3. Keep `kassiber/core/engines/rp2.py` as the only long-term tax-engine adapter in Kassiber
4. Implement Austrian country / accounting / report plugins in `bitcoinaustria/rp2`
5. Use Kassiber's transfer detection, manual pairing, and wallet-bucket preparation to feed the Austrian RP2 path clean inputs
6. Add Austrian-specific report/export code on top of the shared journal output, likely under `kassiber/core/reports.py` or a dedicated report helper once the shape is stable
7. Add any explicit tax-annotation storage only if the normalizer cannot derive legally defensible semantics from existing provenance
8. Expand regression coverage from the current fail-fast checks to the scenario suite above and use it as the parity gate for re-enabling Austrian profiles
9. Add E 1kv CSV/PDF golden tests once report output exists
10. Keep the Steuerberater-review gate/disclaimer when the RP2-backed path becomes runnable

## Open questions

Tracked as a separate document: `07-austrian-tax-open-questions.md`. Defaults applied in this engine are documented there and surfaced in the report footer.

## Out of scope for MVP

- Form E 1kv auto-submission to FinanzOnline (post-MVP)
- Regelbesteuerungsoption computation (user's decision on E 1, not the engine's)
- Business-income crypto (Betriebsvermögen) — MVP covers private sphere only
- NFT and asset-backed-token treatment (kassiber is bitcoin-only)
- DeFi liquidity-mining specific edge cases (kassiber is bitcoin-only)
- Multi-year loss carryforward — explicitly not permitted for crypto under §27a
- Unsupported event semantics are not guessed. They are quarantined until explicit provenance exists.
