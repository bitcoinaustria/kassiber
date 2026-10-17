# Reports and Rates

Use this reference for balances, portfolio views, capital gains, journal exports, PDF export, and exchange-rate sync.

## Output strategy

Preferred defaults:

- `--format plain` for display reports
- `--format csv --output <path>` for export-style reports
- `--format plain reports balance-sheet` for current balances by account, bucket, asset, or wallet
- `--machine reports summary` for exact rollups that should be quoted back without hand math
- `--machine reports tax-summary` for exact yearly gain/loss buckets and totals from RP2
- `--machine reports austrian-e1kv --year <YYYY>` for the Austrian E 1kv handoff envelope
- `reports export-pdf`, `reports export-csv`, or `reports export-xlsx` when the user explicitly asks for a complete report file
- `reports export-year-handoff --year <YYYY>` when the user does bookkeeping (EA, Jahresabschluss) outside Kassiber and needs one year's Bitcoin rows
- `--machine reports cross-check --tool cointracking|blockpit --file <csv> [--year <YYYY>]` when the user asks whether CoinTracking or Blockpit agrees with Kassiber

`--machine`, `--format`, and `--output` are global flags and belong before the subcommand tree. Examples:

```bash
kassiber --format plain reports balance-sheet
kassiber --format csv --output journal-entries.csv reports journal-entries
kassiber --machine reports balance-sheet
```

When parsing programmatically, use `--machine`. Use it alone or with `--format json`; Kassiber rejects any other explicit `--format` value.

## Rates

Rates are cached locally and help fill missing pricing during journal processing:

```bash
kassiber rates pairs
kassiber rates latest BTC-EUR
kassiber rates range BTC-EUR --start 2025-01-01T00:00:00Z --end 2025-01-31T23:59:59Z --order asc
kassiber rates sync --pair BTC-EUR --days 30
kassiber rates sync --source kraken-csv --path ~/Downloads/Kraken_OHLCVT.zip --pair BTC/EUR
kassiber rates sync --source kraken-csv --path ~/Downloads/master_q4 --pair BTC/EUR
kassiber rates set BTC-EUR 2025-01-01T00:00:00Z 95000
```

`rates range --start/--end` expects RFC3339 UTC strings, not Unix epoch
timestamps.

Kassiber's rate cache currently supports `BTC-USD` and `BTC-EUR`. Rate sources
are `coinbase-exchange` (default), `coingecko`, `kraken-csv`, and manual
`rates set` entries. Coinbase Exchange sync stores sparse 1-minute candles from
chunked 300-minute public API windows. When transactions needing pricing exist,
the default sync builds a de-duplicated set of needed transaction minutes,
skips minutes already covered by minute-level cache rows, records checked
Coinbase minutes even when the response is sparse, and fetches only coalesced
300-minute windows around the remaining gaps. With no missing transaction
minutes, `--days` remains a continuous warm-cache fallback. Liquid Bitcoin uses
Kassiber's BTC alias path for fiat pricing, so missing spot prices on LBTC rows
usually mean the relevant BTC sample was unavailable at or before that
timestamp.

For historical minute-level backfills, download Kraken's OHLCVT archive from
[Kraken's support article](https://support.kraken.com/articles/360047124832)
and pass the local ZIP, CSV, or extracted directory with
`--source kraken-csv --path <path>`. Kassiber ingests only 1-minute Bitcoin
pairs for v1, maps Kraken `XBT` filenames to `BTC-USD` / `BTC-EUR`, stores
sparse rows only when Kraken reports a traded candle, and relies on
re-ingesting the latest full or quarterly archive rather than fetching from
Kraken automatically. The desktop Settings → Rate providers panel exposes the
same local ingest as `Full history` and `Incremental update` actions; both use
the idempotent `kraken-csv` upsert path.

When existing provider-derived prices look suspicious, use
`kassiber rates rebuild --source coinbase-exchange --reprice-transactions` or
Settings → Rate providers → Rebuild pricing cache. This clears Coinbase cache
rows, checked-empty minutes, and provider-generated transaction prices before
fetching fresh one-minute samples and reprocessing journals. Manual overrides
and imported exchange execution prices are preserved; large wallets can take a
while because the rebuild refetches provider windows and reruns journal
processing.

If pricing looks incomplete, sync rates and then re-run:

```bash
kassiber rates sync --pair BTC-EUR --days 30
kassiber journals process
```

Rate sync and manual rate overrides mark matching books' journals stale.
Re-run `journals process` before trusting reports after any rate change.

If the user has BTC ↔ LBTC peg-ins / peg-outs or submarine swaps, do not
jump straight from import/sync to reports. Pair those swap legs first:
reports consume the current journal state and do not auto-detect
cross-asset swaps during report generation.

Do not infer the covered history window from `samples` or `days` alone.
Verify actual coverage with `kassiber rates range` around the missing
transaction timestamps. Upstream sources can cap the returned history even when
the sync request asks for more.

## Processed vs Raw

Reports read processed journal state, not raw wallet sync totals.

- quarantined transactions are omitted from processed holdings and gains
- `reports balance-sheet` and `reports portfolio-summary` are the authoritative
  holdings views
- `transactions list` can help estimate raw in/out movement, but that netting
  is only a diagnostic and must not be described as a Kassiber holding
- do not say a BTC ↔ LBTC swap already rolled into reports unless
  `journals transfers list` shows the pair or you just created the pair and
  re-ran `journals process`

## Pagination

Some machine-readable list responses are paginated and keep rows under command-specific keys such as `.data.records` or `.data.events`. Follow `next_cursor` only when the user asked for all/full/export/audit output. For summary, top-N, largest, or smallest questions, use the sorted first page and stop.

## Balance sheet

Use this first for current balances by account, bucket, asset, or wallet. Do
not run `reports summary` first for balance questions.

```bash
kassiber --format plain reports balance-sheet
```

## Summary

Use this first for "what are the totals?" style questions:

```bash
kassiber reports summary
kassiber --machine reports summary
kassiber --machine reports summary --wallet satoshi-liquid
```

This report is the safest source for:

- fee totals
- transaction counts
- priced vs quarantined counts
- holdings cost basis / market value / unrealized PnL
- realized proceeds / cost basis / gain-loss

Prefer the exact fields Kassiber returns. If the payload includes both BTC and `*_msat`, quote those values directly instead of converting them yourself.

## Portfolio summary

```bash
kassiber --format plain reports portfolio-summary
```

When a user asks "what assets do I have?" or "do I still have Liquid balance?",
answer from `reports balance-sheet` or `reports portfolio-summary` first. If
quarantines mean the processed answer differs from raw wallet movement, say so
explicitly and keep any raw transaction-net estimate clearly labeled as an
approximation.

## Tax summary

Use this for yearly gain/loss buckets and totals:

```bash
kassiber --machine reports tax-summary
```

The command emits:

- RP2 yearly detail rows grouped by `year`, `asset`, `transaction_type`, and capital-gains type
- a `year_total` row for each year
- a final `grand_total` row

Total rows only emit quantity when the grouped rows all belong to the same asset. Mixed-asset totals leave quantity blank because cross-asset crypto amounts are not additive.

Prefer these rows over summing `capital-gains` output manually.

## Capital gains

```bash
kassiber --format csv --output capital-gains.csv reports capital-gains
```

## Austrian E 1kv

Use this only for Austrian books (`tax_country=at`, `fiat_currency=EUR`)
after `journals process`:

```bash
kassiber --machine reports austrian-e1kv --year 2024
kassiber --machine reports austrian-tax-summary --year 2024
kassiber --format csv --output e1kv-2024.csv reports austrian-e1kv --year 2024
kassiber reports export-austrian-e1kv-pdf --year 2024 --file e1kv-2024.pdf
kassiber reports export-austrian --year 2024 --file austria-2024.pdf
kassiber reports export-austrian-e1kv-xlsx --year 2024 --file e1kv-2024.xlsx
kassiber reports export-austrian-e1kv-csv --year 2024 --dir e1kv-2024-csv
```

The JSON envelope includes the review gate, the current ausländisch /
self-custody Kennzahl assumption, FinanzOnline summary rows, row-level
details, Steuerbericht-style sections 1.1-4.5, and quarantine/data-quality
notes. `reports austrian-tax-summary` and `reports export-austrian` are aliases
for the same annual Austrian handoff. The CSV output contains the row-level
detail table. The PDF repeats the review gate and assumptions. The XLSX workbook
uses an accountant-facing `Übersicht` sheet, separate numbered section tabs
including `3.3.`, and `Erläuterungen zum Steuerreport`. The CSV bundle mirrors
that layout as separate files so each section keeps its own table shape.

Do not hand-fill domestic-provider or withheld-KESt fields from Kassiber
output today; Kassiber does not yet store the metadata needed for 171, 173, or
175.

## Exit tax (Wegzugsbesteuerung)

```bash
kassiber --format plain reports exit-tax --destination eu_eea
kassiber --machine reports exit-tax --departure-date 2026-07-01 --destination third_country
kassiber reports export-exit-tax-pdf --departure-date 2026-07-01 --file exit-tax.pdf --destination eu_eea
kassiber reports export-exit-tax-xlsx --file exit-tax.xlsx --destination third_country
```

A deemed-disposal estimate of the Austrian exit tax on remaining holdings if the
taxpayer gives up residence on `--departure-date` (defaults to today). Neubestand
unrealized gains are valued at fair market value and taxed at 27.5%; Altbestand
is excluded (tax-free). `--destination eu_eea` reports the tax as assessed but
deferred (Nichtfestsetzung); `third_country` reports it as due immediately. The
amount is the same; only the collection timing differs. Estimate only — it
requires processed journals and is a draft for a Steuerberater, not a filing.
See `docs/plan/11-exit-tax-deemed-disposal.md`.

## Journal entries

```bash
kassiber --format csv --output journal-entries.csv reports journal-entries
```

## Balance history

```bash
kassiber --format plain reports balance-history --interval month
kassiber --format csv --output balance-history.csv reports balance-history --interval week
kassiber --format plain reports balance-history --wallet satoshi-liquid --asset BTC --start 2025-01-01T00:00:00Z --end 2025-12-31T23:59:59Z
```

## Complete report exports

Kassiber includes built-in full-report export commands. The CSV export is a
sectioned spreadsheet-friendly file; the XLSX export uses separate sheets for
overview, wallets, flows, reviewed transfers/swaps, balances, capital gains,
history, data quality, and transactions.

```bash
kassiber reports export-pdf --file report.pdf
kassiber reports export-csv --file report.csv
kassiber reports export-xlsx --file report.xlsx
kassiber reports export-pdf --wallet satoshi-liquid --file satoshi-liquid-report.pdf
```

Use these instead of inventing extra report renderers unless the user asks for
a custom output beyond Kassiber's built-in exports.

### Transactions-only export

For just the transaction ledger (not the full report), use:

```bash
kassiber transactions export --export-format xlsx --file transactions.xlsx
kassiber transactions export --export-format csv --file transactions.csv
kassiber transactions export --wallet satoshi-cold --export-format xlsx --file cold.xlsx
kassiber transactions export --export-format cointracking-csv --file cointracking.csv
kassiber transactions export --wallet satoshi-cold --wallet satoshi-hot --export-format blockpit-csv --file blockpit.csv
kassiber transactions export --period last-month --file last-month.xlsx
kassiber transactions export --from 2025-01-01 --to 2025-03-31 --export-format csv --file q1.csv
```

Repeat `--wallet` to export several wallets together; without it every wallet
is exported. `--period` (today, yesterday, this/last week, month, quarter or
year, or all) or `--from`/`--to` (inclusive days) narrow the rows; days are
counted in the book's calendar (Europe/Vienna for Austrian books, else UTC)
unless `--timezone` names another, and the result reports the resolved
`range`. Transactions without a known date are left out of a period and
counted in `warnings`; an undated quarantine still blocks a platform export.
Without them every transaction is exported. `cointracking-csv` and `blockpit-csv` write another tool's import format
instead of Kassiber's columns; see
[moving to CoinTracking or Blockpit](../../../docs/reference/imports.md#moving-to-cointracking-or-blockpit).
They fail closed on quarantined transactions in scope
(`transaction_export_quarantined`).

The Kassiber `xlsx`/`csv` list is what the wallets observed, not a report.
When the book cannot report yet (an incomplete custody component, unresolved
custody quantity, or journals that need processing) it still exports, as a
draft: `cost_basis` and `gain_loss` stay empty (the pooled average could still
move), `transfer` links stay while journals are current and go when they are
stale, `warnings` names every reason, and the sheet title marks the file a draft. Reports, the year handoff and the CoinTracking/Blockpit
formats stay blocked.

It writes a single styled Transactions sheet (or CSV) with the same columns as
the report's Transactions sheet, so a user can verify each row by hand:

- amount / fee in BTC and msat;
- `fiat_currency`, `fiat_price` (recorded execution/market price per unit),
  `fiat_value` (recorded cash leg — exchange imports fold fiat service fees
  into it), `fmv` (market value of the amount: amount × price when a price is
  recorded, else the recorded value) and `fiat_fee` (the crypto network fee at
  that price; a fiat service fee shows as the `fiat_value` − `fmv` difference);
- `cost_basis` and `gain_loss` — the realized figures from the processed
  journal, restricted to the same disposal/income rows as the capital-gains
  report so each row reconciles against the Capital Detail sheet. Blank for
  rows with no realized figure (e.g. a plain acquisition, or a network-fee
  basis reduction) and blank export-wide while journals are unprocessed or
  stale, so outdated figures are never presented as current;
- `transfer` — for a hop between the user's own wallets, the counterparty
  wallet label (empty for external sends/receipts). With current journals the
  labels come from the booked transfer journal entries themselves (covering
  ownership-derived moves and consolidations too) plus reviewed carrying-value
  swap pairs whose legs were not quarantined; taxable-policy swap pairs are
  never labeled. Without current journals the column is empty, so a blank
  cell then says nothing about whether a move was between own wallets;
- description, note, counterparty, tags, `references` (attachment URLs), and the
  linked-file/URL Attachments column (single URLs render as clickable links).

In the desktop GUI this is the **Export** button on the Transactions screen.
It opens one dialog with a format, a period and a wallet checklist (filter,
All/None, transactions per wallet; every wallet checked means all wallets,
including ones added later). Formats: Kassiber Excel / CSV (daemon kinds
`ui.transactions.export_xlsx` / `ui.transactions.export_csv`), CoinTracking /
Blockpit (`ui.transactions.export_platform_csv` with `platform`), and BIP 329
labels (`ui.metadata.bip329.export`, mode `all`, always all time). The period
is a preset (today … last year, all time) or two days in the book's calendar;
the dialog writes the days its preview resolved. It previews each file before
it is written (`ui.exports.preview`): the real column headers, the first rows,
the row count, the resolved days and any warnings, such as a draft list. A
quarantine block shows there, with the blocking rows and a link to the
Quarantine screen, before export. The year handoff is on the Reports screen
and the CoinTracking/Blockpit comparison on Reconcile, not in this dialog.
It exports the chosen scope, not the screen's
transient view filters. Fiat columns reflect the currency each transaction was
priced in (shown in `fiat_currency`), not a forced conversion.

### Self-verifying XLSX (default)

`reports export-xlsx` appends a verification layer so the reader can reproduce
every figure in Excel/LibreOffice rather than trusting the static numbers:

- **Acquisitions** / **Disposals** — the raw journal ledger. Only the
  highlighted inputs (msat quantities, fiat values, proceeds, cost basis) are
  hard numbers; quantities-in-BTC, per-row gain = proceeds − cost basis, and an
  OK/DIFF check are live formulas. Each row also shows its **Pricing Source**
  and **Pricing Quality** (coarse/estimated prices are highlighted).
- **Control** — a per-asset reconciliation matrix. Holdings balance, cost
  basis, average price, market value, unrealized and realized gain are each
  recomputed with a live formula over the ledger sheets and shown next to
  Kassiber's own number with an OK/DIFF check (the check references the editable
  tolerance in `Verify!B3`). It also shows the **market rate, its source and
  timestamp** so the valuation is traceable.
- **Verify** — a plain-language "how to verify" sheet: a workbook-level
  `ALL CHECKS OK` / mismatch banner, run metadata (lot method, fiat, last
  processed, Kassiber version), the formula legend, the recalc gotcha, the
  active lot method, and scope notes.
- **Quarantined** (only when present) — the transactions Kassiber could not
  classify, with reason and detail, so the reader can see what is deliberately
  excluded from every figure.

The main report's **Transactions** sheet is the full per-transaction record:
description, note, counterparty, tags, and an **Attachments** column. A single
linked URL renders as a clickable link shown behind its name; multiple
attachments are listed one per line (Excel allows only one hyperlink per cell).
The Acquisitions/Disposals ledgers also carry each row's description and tags;
match a ledger row to its evidence by the Transaction ID. When any attachments
exist, an **Evidence** sheet lists every link as its own row with a clickable
styled link — so even a transaction with several links has each one clickable.

Reconciliation is per asset across the whole profile (Bitcoin accounting is
pooled per asset across wallets; per-wallet cost basis is an allocation).
Per-disposal cost basis under FIFO/LIFO/HIFO/LOFO is engine-selected and cannot
be re-derived by a plain formula — the Control sheet verifies the
method-independent identities instead. Pass `--no-verify` for the lean
value-only workbook. The daemon kind `ui.reports.export_xlsx` accepts
`{"verify": false}` for the same effect.

## Year handoff for bookkeeping

For an Einnahmen-Ausgaben-Rechnung or Jahresabschluss kept in a spreadsheet or
by an accountant, export one year's Bitcoin side after `journals process`:

```bash
kassiber reports export-year-handoff --year 2025 --file handoff-2025.xlsx
```

Austrian books use the Europe/Vienna calendar year; other books use UTC. The
workbook has eight sheets:

- **Overview**: scope, assets with their ids, run metadata, reconciliation
  status, warnings, counts, and a count per classification.
- **Wallets**: every wallet in the book with kind, chain, network, status,
  assets, transactions and exclusions in the year, first/last transaction, and
  last sync. Wallets in other books are not included; a transfer to one of them
  reads as an external payment.
- **Transactions**: every non-excluded transaction observed in the year:
  wallet, kind, chain and network; `Transaction Ref` (Kassiber's id) and
  `Transaction ID`; direction; `Kassiber Classification` from the processed
  journal (`internal_transfer`, `swap`, `income`, `acquisition`, `disposal`,
  `fee`, or `no_journal_entry`; several are joined with `+`) next to the
  importer's `Source Kind`; `Pair Ref`, `Other Wallet` and
  `Other Leg Transaction ID`, shared by both legs of an own-wallet transfer,
  reviewed swap, or swap payout; amount, fee and signed net change in the
  asset's own unit, as lossless `(exact)` text, and in sats for BTC and L-BTC
  only; recorded price and values plus the journal's income, proceeds,
  disposed basis, gain and fee basis, all in the book currency named in the
  header; tags, notes and attachments.
- **Journal**: every journal entry booked in the year with the same
  `Transaction Ref`. Descriptions omit internal `at_*=` markers;
  `In Capital Gains Report` shows whether the row counts in
  `reports capital-gains`.
- **Balances**: opening and closing quantity and cost basis per wallet and
  asset, valued at the market rate at year end with its source and timestamp.
  Per-wallet basis is the pooled per-asset basis allocated by quantity.
- **Roll-forward**: per asset, opening + acquired − disposed − fees + own-wallet
  transfers = closing, in units and book currency, with live check formulas
  that must be 0.
- **Reconciliation**: per wallet and asset, journal balances against observed
  wallet movements at both ends of the year, with live formulas. A `DIFF` row
  needs an explanation; `Timing Notes` names rows the journal books on the
  other side of a year boundary from when they were observed (for example a
  transfer that arrives on 1 January is booked at its send time).
- **Excluded**: transactions excluded from the books in the year, with the
  exclusion reason and date. They count nowhere else.

Assets are grouped by canonical identity (`bitcoin:btc`, `liquid:lbtc`,
`liquid:<asset-id>`, or a namespaced extension id), never by display code, so a
ticker shared across chains cannot merge. Issued Liquid assets show their short
asset id because Kassiber does not hold a ticker registry.

The export fails closed like every report, and also refuses to run while any
quarantine dated in or before the year is unresolved. A year-end value that
falls back to a transaction price, or has no rate at all, appears as a
warning; it is never valued at zero. Kassiber does not assign spheres,
accounts, or the valuation rule. Tags are the way to carry that assignment
into the workbook.

In the desktop GUI, the **Reports** screen lists the handoff with the other
files for the selected tax year. It calls the daemon kind
`ui.reports.export_year_handoff` with `{"year": <YYYY>}`.

## Cross-check against CoinTracking or Blockpit

```bash
kassiber --machine reports cross-check --tool blockpit --file blockpit.csv --year 2025
```

Reads the tool's transaction CSV without importing it and pairs its rows with
the book's (transaction id first, then exact amount within a day). `counts`
splits rows into `different`, `only_tool`, `only_kassiber`,
`excluded_in_kassiber`, `matched` and `not_comparable`; `tax_differences`
counts differences that change a tax result (amount, value, type, tax year),
while fee splits and time drift are informational. `pairs` maps each Kassiber
wallet to the tool's name for it and names wallets only one side has. `totals` compares received,
sent, fees and income value per asset; `insights` names a clock offset, stale
journals (types not compared), skipped wallets imported from the same tool,
and uncompared assets. Quote rows by their `kassiber.ref` / `tool.row`. The
**Reconcile** screen's CoinTracking & Blockpit mode shows the same result; `--xlsx` writes it.

## Source of funds (Mittelherkunftsnachweis)

Target-anchored proof-of-source workflow: pick the outgoing target
transaction (e.g. a planned exchange sale), link upstream funding evidence,
review gates, then export a PDF rendered only from a saved immutable case
snapshot.

```bash
# Build/maintain the review graph.
kassiber source-funds sources create --type fiat_purchase --label "Bank purchase" \
  --asset BTC --amount 0.10000000 --attachment <attachment-id>
kassiber source-funds links create --from-source <source-id> \
  --to-transaction <txid-or-id> --type manual_source \
  --allocation-amount 0.10000000 --allocation-policy explicit
# One call: derive + auto-review everything provable from local evidence
# (scoped tx inputs/outputs, source-qualified Lightning hashes, reviewed pairs).
# Provider/platform ids remain manual suggestions.
kassiber source-funds assemble --target-transaction <txid-or-id>

# Manual equivalents when finer control is needed:
kassiber source-funds suggest --target-transaction <txid-or-id>
kassiber source-funds links bulk-review --target-transaction <txid-or-id>

# Preview gates + disclosure, freeze a case, then export it.
kassiber --machine reports source-funds --target-transaction <txid-or-id> \
  --target-amount 0.10000000 --reveal-mode standard --save-case
kassiber reports export-source-funds-pdf --case <case-id> --file sof.pdf
kassiber reports export-source-funds-bundle --case <case-id> --file sof-bundle.zip
```

For agent-led work, begin with `ui.source_funds.review_context` for the exact
target instead of loading whole-book coverage. It returns the canonical report,
reachable links/sources/evidence, documentary `input_needs`, and a
`review_fingerprint` binding the current provenance facts and complete `recipe`.
Respect `scope_truncated`; historical inbound coverage is not current holdings.
Each link carries exact `allocation_amount_msat` / `from_allocation_amount_msat`;
review a link by sending only what the user actually asked to change, and pass
the inspected values as `expected_allocation_amount_msat` /
`expected_from_allocation_amount_msat` so an approval cannot land on amounts that
moved. Omitting an allocation keeps the stored one, and a rejection may not
carry an allocation edit. `bulk_review.eligible_link_ids` is the only batch
eligibility verdict; pass it as `link_ids` to bind an apply to what you read.
CLI equivalents are `source-funds review-context --target-transaction <id>` and
`source-funds request-input --target-transaction <id> --action <action>
--expected-review-fingerprint <fingerprint> --recipe-file <context.json>`.

When evidence is missing, use `ui.source_funds.request_input` with the unchanged
fingerprint and canonical recipe, explain the missing facts in ordinary words,
and wait for the user's connection/import/evidence step. This works outside
quarantine and can improve an exportable attestation. The native evidence picker
attaches the selected file durably to the target and stages that same managed
copy for analysis. An attachment is evidence to review, not proof of origin or
permission to change tax/custody interpretation. Re-read current context after
input, assembly, source/link edits, or evidence changes; do not reuse stale facts.

Before saving, review `explain_gates` and `disclosure_preview`, then pass the full
canonical recipe and `expected_review_fingerprint` to `ui.source_funds.cases.save`
with consent. Reuse its exact saved case ID for export and check the returned
snapshot hash and file outcome. A saved snapshot remains historical; it does not
assert that current facts are unchanged. A file/export failure is not completion,
and missing or changed originals must not be described as verified evidence.
Private URL/path-bearing recipes are rejected on the AI boundary rather than
silently redacted into an unusable fingerprint. Use a destination label and
non-sensitive note, or the local UI/CLI. Recipes too large for the bounded chat
context also require a narrower recipe or the local workflow.

Gotchas:

- `export-source-funds-pdf` / `export-source-funds-bundle` take ONLY
  `--case` + `--file`: target, reveal mode, and report options are frozen
  into the snapshot at `--save-case` time and cannot be reshaped at export.
- Export hard-fails (`export_blocked`) while `explain_gates.blockers` is
  non-empty; there is no force flag. Resolve blockers (review suggested
  links, fix allocations, attach missing-history attestations) instead.
- Reveal modes: `labels_only`, `minimal`, `standard`, `full`. Free-text and
  txids tighten as the mode narrows; `disclosure_preview` in the machine
  envelope lists exactly what would be disclosed, the wallets the report
  names, and the common-ownership consequence of sharing it.
- The machine envelope carries the granular trace: `flow_levels` (per-level
  nodes with direction, fee, fiat value, `data_provenance` =
  chain_sync/platform_export/manual_import, per-level fiat subtotals),
  `data_provenance_summary`, `source_mix`, `gaps` (missing-history items
  carry the unexplained amount), and `findings`.
