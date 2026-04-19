# Reports and Rates

Use this reference for balances, portfolio views, capital gains, journal exports, PDF export, and exchange-rate sync.

## Output strategy

Preferred defaults:

- `--format plain` for display reports
- `--format csv --output <path>` for export-style reports
- `reports export-pdf` only when the user explicitly asks for a PDF

When parsing programmatically, use `--machine`.

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

## Balance sheet

```bash
kassiber reports balance-sheet --format plain
```

## Portfolio summary

```bash
kassiber reports portfolio-summary --format plain
```

## Capital gains

```bash
kassiber reports capital-gains --format csv --output capital-gains.csv
```

## Journal entries

```bash
kassiber reports journal-entries --format csv --output journal-entries.csv
```

## Balance history

```bash
kassiber reports balance-history --format plain --interval month
kassiber reports balance-history --format csv --output balance-history.csv --interval week
kassiber reports balance-history --wallet satoshi-liquid --asset BTC --start 2025-01-01T00:00:00Z --end 2025-12-31T23:59:59Z
```

## PDF export

Kassiber includes a built-in PDF export command:

```bash
kassiber reports export-pdf --file report.pdf
kassiber reports export-pdf --wallet satoshi-liquid --file satoshi-liquid-report.pdf
```

Use this instead of inventing extra report renderers unless the user asks for a custom output beyond Kassiber's built-in PDF.
