# Reports and Rates

Use this reference for balances, portfolio views, capital gains, journal exports, PDF export, and exchange-rate sync.

## Output strategy

Preferred defaults:

- `--format plain` for display reports
- `--format csv --output <path>` for export-style reports
- `--machine reports summary` for exact rollups that should be quoted back without hand math
- `--machine reports tax-summary` for exact yearly gain/loss buckets and totals from RP2
- `reports export-pdf` only when the user explicitly asks for a PDF

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
kassiber rates range BTC-EUR --start 2025-01-01T00:00:00Z --end 2025-01-31T23:59:59Z
kassiber rates sync --pair BTC-EUR --days 30
kassiber rates set BTC-EUR 2025-01-01T00:00:00Z 95000
```

If pricing looks incomplete, sync rates and then re-run:

```bash
kassiber journals process
```

If the user has BTC ↔ LBTC peg-ins / peg-outs or submarine swaps, do not
jump straight from import/sync to reports. Pair those swap legs first:
reports consume the current journal state and do not auto-detect
cross-asset swaps during report generation.

## Pagination

Some machine-readable list responses are paginated and keep rows under command-specific keys such as `.data.records` or `.data.events`. When `next_cursor` is present, keep requesting more pages until it becomes `null`.

## Balance sheet

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

## PDF export

Kassiber includes a built-in PDF export command:

```bash
kassiber reports export-pdf --file report.pdf
kassiber reports export-pdf --wallet satoshi-liquid --file satoshi-liquid-report.pdf
```

Use this instead of inventing extra report renderers unless the user asks for a custom output beyond Kassiber's built-in PDF.
