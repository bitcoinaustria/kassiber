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
```

It writes a single styled Transactions sheet (or CSV) with the same columns as
the report's Transactions sheet — description, note, counterparty, tags, and the
linked-file/URL Attachments column (single URLs render as clickable links). In
the desktop GUI this is the **Export** button on the Transactions screen toolbar
(Excel / CSV), backed by the daemon kinds `ui.transactions.export_csv` /
`ui.transactions.export_xlsx`. It exports the profile's transactions (wallet
scope when given), not the screen's transient view filters.

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
