# Austrian Tax Engine — Design

**Status:** An experimental Austrian journal engine now exists for supported acquisitions, disposals, and self-transfers. Unsupported or ambiguous provenance quarantines instead of being guessed, and Austrian JSON report envelopes carry explicit review markers; E 1kv export is still pending.
**Module:** `kassiber/core/engines/austria.py`
**Report:** Planned E 1kv export layered on top of the shared journal/report pipeline.
**Legal gate:** Output requires Steuerberater review before filing. A disclaimer surfaces on first use and on every report.

## Why a separate engine and not an RP2 config

RP2 is excellent for US-style tax regimes (lot-based FIFO/LIFO/HIFO/LOFO with day-count holding periods). The Austrian regime differs structurally in three ways that RP2 does not model:

1. **Cost basis from 2023-01-01 requires gleitender Durchschnittspreis** (moving average). RP2 has no moving-average engine — all its accounting methods are lot-tracking.
2. **Crypto-to-crypto swaps are non-taxable for Neuvermögen** under §27b Abs 3 Z 2 EStG, with basis carrying to the new asset. RP2 treats every disposal as taxable.
3. **Regime classification is by acquisition date, not holding period.** Coins acquired on/before 2021-02-28 are Altvermögen (old regime); after are Neuvermögen (new regime). RP2's `long_term_days` is a days-threshold, not a calendar cutoff.

Additionally, the Altvermögen rules themselves involve a 1-year Spekulationsfrist that resembles RP2's long-term threshold but only applies to the Altvermögen tranche and expires via swap, which adds state RP2 doesn't track.

The cleanest path is a **separate engine** sharing Kassiber's current `TaxEngine` interface with `kassiber/core/engines/rp2.py`. Both engines should consume the same per-profile ledger inputs; users select one via tax policy.

## Prerequisite: normalization and provenance layer

Before the Austrian engine can be trusted, Kassiber needs a tax-input normalization seam between raw transactions and tax-engine logic.

Why:

- Raw `transactions` today do not reliably distinguish buy vs mining vs routing income vs inheritance vs swap.
- Manual transfer pairs and cross-asset pair audit state already affect tax behavior and need to be explicit inputs.
- Some Austrian questions require facts Kassiber may not currently observe; those cases must be quarantined or explicitly annotated rather than guessed.

Phase 0.5 therefore introduces `kassiber/core/tax_events.py`:

- Input: raw transactions, wallet metadata, transfer-pair state, explicit tax annotations, and rate lookup helpers.
- Output: typed `NormalizedTaxEvent` records for the engine.
- Rule: if the normalizer cannot prove the event type with acceptable confidence, it emits an ambiguous event and the engine quarantines it.

This layer is shared by both the generic RP2 engine and the future Austrian engine, so the Austrian work improves the generic engine boundary instead of forking the ingestion story.

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
- **No new wallet Altbestand column in Phase 0.5.** The existing wallet-level Altbestand provenance stays in `wallets.config_json` until there is a deliberate migration away from that contract.

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

The future Austrian engine should fit this same boundary. Austrian-specific summaries such as E 1kv totals belong in a reporting layer built from the shared journal state, not in a separate engine-only return type.

## Algorithm — full pipeline

Pseudocode for a future Austrian engine `build_ledger_state()`:

```
# 1. Start from the shared per-profile journal inputs
rows = inputs.rows
manual_pairs = inputs.manual_pair_records
wallet_refs = inputs.wallet_refs_by_id

# 2. Reuse the same transfer detection/manual-pair story as the generic engine,
#    then normalize per asset through kassiber/core/tax_events.py
for asset, asset_rows in group_by_asset(rows):
    normalized = normalize_tax_asset_inputs(profile, asset, asset_rows, wallet_refs, pairs_for_asset)
    events = sort(normalized.events, key=(occurred_at, transaction_id))

# 3. Classify every acquisition-like event
for e in events:
    if e.kind is acquisition_like:
        e.at_regime = classify(e)
    # disposals inherit regime from the lot(s) they disposed

# 4. Per-container running state
state = {container_id: WalletState() for container_id in containers}

# WalletState tracks:
#   - altvermögen lots: list[AltLot(qty_msat, cost_eur_cents, acquired_at)]
#   - neuvermögen pre-2023 lots: list[NeuLot(qty_msat, cost_eur_cents, acquired_at)]
#   - neuvermögen from-2023 running avg: (qty_msat, avg_price_eur_cents_per_btc)

# 5. Iterate events
for e in events:
    entries_out, quarantine_reason = process(e, state[e.container_id], normalized.transfers)
    journal.extend(entries_out)
    if quarantine_reason:
        quarantined.append((e.tx_id, quarantine_reason))

# 6. Return shared ledger state; Austrian E 1kv summaries derive from journal output later
```

### `process()` per event kind

**Acquisition (buy, receive from external, mining, airdrop, lending income):**
- Determine regime (above)
- If regime == 'altvermoegen': append `AltLot(qty, cost=eur_at_fmv, acquired_at=e.timestamp)` to wallet state
- If regime == 'neuvermoegen' pre-2023: append `NeuLot(...)` to wallet state
- If regime == 'neuvermoegen' from-2023: update running `(qty, avg_price)`
- For income events (mining, staking, airdrops, lending, LN routing): emit an `income` JournalEntry with `income_eur_cents=FMV` (cost=0 for airdrops/staking)

**Disposal (sell, spend, BTC → altcoin if treated as disposal under Altvermögen):**
- If the container's earliest lots are Altvermögen: apply FIFO within Altvermögen, determine holding period, emit `disposal` entry. If >1y, emit with `gain_loss = 0` (tax-free note in the entry); if ≤1y, emit with real gain and `at_regime='altvermoegen'` (user handles progressive rate externally)
- If the container's earliest lots are Neuvermögen pre-2023: FIFO within NeuLots, emit disposal at 27.5% path
- If the container's current regime is Neuvermögen from-2023: consume qty from running state, emit disposal at 27.5% path. Average unchanged.

**Self-transfer (same-owner movement, outgoing leg):**
- No journal entry
- Move qty to destination container:
  - If Altvermögen: preserve per-lot breakdown (carry acquired_at and cost intact)
  - If Neuvermögen from-2023: update destination's running average per the rule (inherit source's avg_price for the transferred qty)
  - If there is a transfer fee, emit the AT equivalent of today's `transfer_fee` disposal treatment rather than dropping the fee on the floor

**Missing price:**
- Quarantine with reason: `"no EUR rate available at timestamp"`

**Ambiguous regime or unsupported provenance:**
- Quarantine with reason such as `"regime classification failed"` or `"insufficient tax provenance"` and surface in report

### Loss treatment

- Losses within Neuvermögen offset gains within Neuvermögen, same tax year, same regime
- No carryforward (per § 27 EStG as applied to crypto since the reform)
- Losses within Altvermögen don't offset Neuvermögen gains (different regime)
- The engine surfaces all gains and losses separately; offsetting is applied in the summary and surfaced in the E 1kv output

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

> Kassiber's Austrian tax engine is a self-help tool. It does not constitute tax advice.
>
> The output is designed to support your preparation of the E 1kv form but must be reviewed by a Steuerberater before filing.
>
> Kassiber maintains a list of genuinely unsettled questions in the Austrian crypto tax landscape (see `docs/plan/07-austrian-tax-open-questions.md`). The engine applies a reasonable default for each; your Steuerberater may instruct a different treatment.

A footer on every E 1kv PDF repeats the Steuerberater-review gate and lists any open-question defaults used in that report.

## Policy registration

```python
# kassiber/tax_policy.py
def build_austrian_policy(profile):
    return TaxPolicy(
        tax_country="at",
        fiat_currency="EUR",
        long_term_days=365,
        accounting_methods=("fifo", "lifo", "hifo", "lofo"),
        report_generators=("open_positions", "rp2_full_report"),
    )


# kassiber/core/engines/__init__.py
def build_tax_engine(profile):
    if normalize_tax_country(profile_value(profile, "tax_country")) == "at":
        return ExperimentalAustrianTaxEngine(profile)
    return GenericRP2TaxEngine(profile)
```

Today the policy registration and the first Austrian ledger builder both live behind `ExperimentalAustrianTaxEngine`. The engine runs on the shared ledger seam, but it remains conservative: unsupported semantics quarantine until Kassiber has stronger provenance capture.

## Testing

Current coverage in `tests/test_review_regressions.py` verifies:

- Austrian profiles normalize to `tax_country="at"`, `fiat_currency="EUR"`, and `tax_long_term_days=365`
- supported Austrian journal flows process successfully through the shared ledger seam
- unsupported inbound kinds such as `lightning_received` quarantine as `insufficient_tax_provenance`
- switching an existing profile to Austrian invalidates stale journals before reprocessing

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
3. Keep `kassiber/core/engines/rp2.py` as the generic reference implementation for the shared seam
4. Replace the `ExperimentalAustrianTaxEngine` gate in `kassiber/core/engines/austria.py` with the real Austrian ledger builder
5. Add Austrian-specific report/export code on top of the shared journal output, likely under `kassiber/core/reports.py` or a dedicated report helper once the shape is stable
6. Add any explicit tax-annotation storage only if the normalizer cannot derive legally defensible semantics from existing provenance
7. Expand regression coverage from the current gating checks to the scenario suite above
8. Add E 1kv CSV/PDF golden tests once report output exists
9. Keep the Steuerberater-review gate/disclaimer when the engine becomes runnable

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
