# Reports and Rates

Use this reference for balances, portfolio views, capital gains, journal exports, PDF export, and exchange-rate sync.

## Output strategy

Preferred defaults:

- `--format plain` for display reports
- `--format csv --output <path>` for export-style reports
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

## Portfolio summary

```bash
kassiber --format plain reports portfolio-summary
```

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
