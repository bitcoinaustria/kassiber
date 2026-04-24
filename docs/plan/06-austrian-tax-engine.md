# Austrian Tax Support On RP2

**Status:** Active RP2-backed processing; review-gated E 1kv CSV/PDF/XLSX export
is implemented for the current ausländisch / self-custody slice.
**Current source of truth:** `docs/austrian-handoff.md`,
`kassiber/core/tax_events.py`, `kassiber/core/engines/rp2.py`,
`kassiber/core/austrian.py`, `tests/test_review_regressions.py`, and TODO.md.
**Legal gate:** Kassiber is not tax advice. Austrian output must be reviewed by
a Steuerberater before filing.

## Product Boundary

Kassiber is the local-first Bitcoin accounting and reconciliation layer. RP2 is
the tax engine.

Kassiber owns:

- wallet sync/import, rates, provenance, notes/tags/exclusions, attachments
- transfer and manual pair preparation
- tax-input normalization and quarantine UX
- persisted journal/report rows and desktop/CLI presentation
- Austrian report packaging such as E 1kv

RP2 / `bitcoinaustria/rp2` owns:

- country tax semantics
- accounting methods and lot/moving-average math
- gains/losses and disposal classification
- Austrian plugin APIs such as `rp2.plugin.country.at.AT` and
  `classify_disposal()`

Do not grow a second Austrian tax engine inside Kassiber.

## Austrian Rules Kassiber Must Respect

- Austrian profiles use `tax_country=at`.
- Acquisitions before `2021-03-01 Europe/Vienna` are Altvermögen; later
  acquisitions are Neuvermögen.
- Neuvermögen from 2023 uses moving average where supported by the RP2 fork.
- Crypto-to-crypto swaps can be non-taxable for Neuvermögen, with basis carried
  to the acquired asset.
- Cross-asset carrying-value pairing is Austrian-only today. Generic profiles
  keep BTC/LBTC pairs as audit-linked SELL + BUY.
- Missing prices, malformed transfers, or ambiguous tax semantics quarantine
  instead of being guessed.
- Raw `transactions` remain the source of truth; derived Austrian regime state
  is not written back onto them.

## Normalization Contract

`kassiber/core/tax_events.py` turns raw transaction rows plus wallet metadata,
manual pairs, rates, and explicit annotations into typed normalized events.

A **normalized tax container** is the unit used for Austrian basis/pool logic.
The current binding is one container per `wallet_id`. Future address/UTXO-level
provenance can narrow that binding without changing the engine boundary.

Typed Austrian fields on normalized events:

- `at_regime`
- `at_pool`
- `at_swap_link`
- `carried_basis_fiat`

The RP2 adapter serializes those markers into RP2 notes per
`docs/austrian-handoff.md`.

## Current Engine Boundary

The shared engine seam is intentionally narrow:

```python
TaxEngine.build_ledger_state(inputs: TaxEngineLedgerInputs) -> TaxEngineLedgerResult
```

`TaxEngineLedgerResult` returns journal entries, quarantines, transfer audit
data, cross-asset pair data, and holdings. Austrian-specific report summaries
should be built from the persisted journal state, not returned as a separate
engine type.

## Current Persisted AT Mapping

`journal_entries` already stores:

- `at_category`
- `at_kennzahl`

Presentation-layer mapping lives in `kassiber/core/austrian.py`.

E 1kv export is built from these persisted journal rows plus profile and
quarantine state. The first implementation lives in the report builder module:

```text
kassiber/core/reports.py
```

Split it into `kassiber/core/reports/e1kv.py` only when that removes real
complexity.

## Possible Future Annotation Table

Do not add this until the normalizer needs explicit user semantics that cannot
be represented by existing metadata, tags/notes, manual pairs, or quarantines.

```sql
CREATE TABLE transaction_tax_annotations (
    transaction_id    TEXT PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
    event_type        TEXT,
    provenance_json   TEXT NOT NULL DEFAULT '{}',
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
```

For new Austrian report-specific persisted money amounts, prefer integer
eurocents and define the rounding boundary explicitly. Existing transaction and
journal fiat columns are still `REAL`.

## E 1kv Export

Implemented surfaces:

- `reports austrian-e1kv --year <YYYY>` for structured JSON/plain output
- `reports austrian-tax-summary --year <YYYY>` as a friendlier alias for the
  same structured annual handoff
- `--format csv --output <path> reports austrian-e1kv --year <YYYY>` for
  row-level CSV
- `reports export-austrian-e1kv-pdf --year <YYYY> --file <path>` for the
  PDF handoff
- `reports export-austrian --year <YYYY> --file <path>` as a friendlier alias
  for the same PDF handoff
- `reports export-austrian-e1kv-xlsx --year <YYYY> --file <path>` for the
  styled workbook handoff
- Steuerbericht-style sections 1.1-4.5 in the structured output and PDF, with
  unsupported sections rendered as explicit zero-value placeholders
- CoinTracking-style XLSX workbook layout with `Übersicht`, separate numbered
  section tabs including `3.3.`, and `Erläuterungen zum Steuerreport`
- CLI/PDF/XLSX review gate and current ausländisch / self-custody assumption
- regression coverage for JSON, CSV, PDF, and XLSX generation

CSV contains one row per relevant Austrian journal entry:

```text
tax_year,date,tx_id,transaction_id,wallet,asset,kind,entry_type,at_category,at_category_label,at_regime,qty_msat,quantity,price_eur_cents,cost_basis_eur_cents,proceeds_eur_cents,gain_loss_eur_cents,income_eur_cents,form_amount_eur_cents,holding_period_days,kennzahl,stored_kennzahl,form_section,note
```

PDF uses Kassiber's existing line-oriented PDF writer. XLSX uses XlsxWriter as
a small write-only dependency and lays out the workbook as an overview, numbered
tax-section sheets, and an explanatory notes sheet. Both exports repeat the
Steuerberater-review gate and list the invoked assumptions / open-question
defaults.

## Test Direction

Keep growing coverage around:

- AT profiles processing through `rp2.plugin.country.at.AT`
- persisted `at_category` / Kennzahl mapping
- Neu cross-asset carrying-value basis carry
- staking/income-like receipts
- missing-price and ambiguous-semantics quarantines
- Alt/Neu classification edges
- E 1kv CSV/PDF/XLSX output

The existing regression and snapshot tests are the gate; expand them rather than
adding unpinned behavior.

## RP2 Fork Risk

The fork solves upstream stagnation but creates divergence risk. Keep the
Kassiber adapter small, pin the fork intentionally, and periodically upstream or
rebase Austrian primitives where practical. Do not let Kassiber-side report
needs leak tax math back across the seam.

## Out Of Scope

- FinanzOnline auto-submission
- Regelbesteuerungsoption computation
- Betriebsvermögen/business-income tax engine behavior
- NFT, DeFi, and asset-backed-token support
- altcoin product scope beyond defensive tax-engine boundaries
- multi-year crypto loss carryforward
