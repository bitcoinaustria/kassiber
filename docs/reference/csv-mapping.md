# Generic CSV import (auto-detect + mapping)

Kassiber ships dedicated importers for known wallets/exchanges (Phoenix, River,
BullBitcoin, Coinfinity, 21bitcoin, Pocket, Strike, BTCPay, Wasabi, Samourai).
For everything else there is the **generic CSV import**: download a fill-in
example, paste your transactions into it (or use your own export), and Kassiber
**auto-detects the columns** and imports. Under the hood the detector builds a
declarative mapping spec; you can also supply that spec by hand for unusual
files. It turns "my exchange isn't supported" into "download, fill, import."

Engine: [`kassiber/core/csv_mapping.py`](../../kassiber/core/csv_mapping.py)
(pure, standard-library, `Decimal`-exact). The same engine powers the CLI and
the desktop **CSV import** connection in the Add Connection modal.

## CLI

```sh
# 1. Write a fill-in example (canonical-friendly headers + sample rows)
kassiber wallets csv-example --file import.csv      # then fill it in

# 2. Preview WITHOUT a mapping — columns are auto-detected
kassiber wallets import-mapped-csv \
  --workspace Main --profile Default --wallet "My Exchange" \
  --file import.csv --dry-run

# 3. Import for real (re-running the same file is idempotent)
kassiber wallets import-mapped-csv \
  --workspace Main --profile Default --wallet "My Exchange" \
  --file import.csv
```

Omit `--mapping` to auto-detect (the primary path). If the columns can't be
recognized you get a `csv_mapping_unrecognized` error pointing you at the
example file; for unusual files, pass an explicit `--mapping` (a path to a JSON
spec or an inline JSON object — `kassiber wallets mapping-template` prints a
starter). `--dry-run` returns `{confident, detected[], mapped, errors, filtered,
problems[], preview[]}` and persists nothing; `--limit N` bounds the preview.

## Auto-detection

`infer_mapping(headers)` matches column headers (case-insensitive, English +
common German aliases) to canonical fields: date/timestamp, amount (or split
sent/received, or amount + a direction column), fee, transaction id, note, kind,
counterparty, and fiat currency/price/value. Detection is **confident** only when
a date and an amount layout are recognized; otherwise nothing is imported. The
example template uses unambiguous headers so it always detects cleanly.

## The mapping spec

```jsonc
{
  "version": 1,
  "name": "My Exchange",
  "asset": "BTC",                 // BTC or LBTC
  "delimiter": null,               // null = auto-detect; or "," ";" "\t" "|"
  "encoding": "utf-8-sig",         // BOM-safe default
  "skip_rows": 0,                  // raw lines to drop *before* the header row

  "timestamp": {                   // required
    "column": "Date",
    "format": null,                // null = flexible ISO; else strptime, e.g. "%d.%m.%Y %H:%M"
    "timezone": "UTC"              // IANA name; applied to naive timestamps
  },

  "amount": {                      // required — pick one mode
    "mode": "signed",              // "signed" | "split" | "absolute"
    "column": "Amount",            // signed/absolute
    "inbound_column": "Received",  // split
    "outbound_column": "Sent",     // split
    "unit": "btc",                 // btc | sat | msat
    "decimal_separator": ".",      // "." (1,234.56) or "," (1.234,56)
    "direction": {                 // absolute mode only
      "column": "Type",
      "inbound_values": ["deposit", "buy"],
      "outbound_values": ["withdrawal", "sell"],
      "default": null              // null => unmatched rows error
    }
  },

  "fee":  { "column": "Fee", "unit": "btc" },     // optional
  "txid": { "column": "TxHash" },                  // optional (see "Identity")

  "fields": {                                       // optional metadata
    "kind":         { "column": "Type" },
    "description":  { "column": "Note" },
    "counterparty": { "const": "Acme Exchange" }    // {column} or {const}
  },

  "pricing": {                                      // optional fiat enrichment
    "fiat_currency": { "const": "EUR" },
    "fiat_rate":     { "column": "Price" },
    "fiat_value":    { "column": "Total" },
    "source_kind":   "generic_import",
    "decimal_separator": "."
  },

  "filters": [                                      // keep only matching rows
    { "column": "Asset", "op": "equals", "value": "BTC" }
  ]
}
```

### Amount modes

- **signed** — one column whose sign encodes direction (`-0.1` → outbound).
- **split** — separate inbound/outbound columns; both non-empty → `split_ambiguous`,
  neither → `amount_missing`.
- **absolute** — a magnitude column plus a `direction` (a constant, or a column
  matched case-insensitively against `inbound_values` / `outbound_values`).

### Filters

Each filter is `{column, op, value?}` with `op` ∈ `equals | in | not_empty`
(comparisons are case-insensitive). Rows that fail a filter are **skipped**
(counted as `filtered`), not errored — use this to keep only Bitcoin rows.

### Identity and idempotency

If `txid` is unmapped or blank for a row, the engine synthesizes a stable id
(`csvmap:<hash>` from the row's position + content). This keeps two distinct
same-day / same-amount rows separate **and** keeps re-importing the same file
idempotent (the dedupe fingerprint keys on the id). Map a real transaction-id or
reference column whenever the export has one — that is the most robust dedupe.

### Row problems

`apply_mapping` never aborts on row data; each row yields either a record or one
problem with a stable machine code the desktop localizes: `amount_missing`,
`bad_amount`, `bad_fee`, `bad_timestamp`, `split_ambiguous`,
`direction_unresolved`, `filtered_equals`, `filtered_in`, `filtered_not_empty`.

## Desktop

The **CSV import** connection (Add Connection → Files → CSV import) embeds the
whole flow in the modal ([`CsvImportPanel`](../../ui-tauri/src/components/kb/csv-mapping/CsvImportPanel.tsx)):

1. **Download example CSV** — `ui.wallets.csv_example` writes the fill-in
   template; the user pastes their transactions into it (or uses their own file).
2. **Choose file** — picking the file runs `ui.wallets.csv_preview` with no
   mapping, which auto-detects the columns and returns `{confident, detected[],
   preview[], …}`.
3. **Preview + confirm** — the detected columns and a row preview are shown; the
   primary action is **Import N rows** (`ui.wallets.import_mapped_csv`, which
   creates a `custom` wallet then imports).
4. **Fallback** — when detection isn't confident the user is steered to the
   example, and an **Advanced** expander surfaces the full column-mapping editor
   ([`MappingControls`](../../ui-tauri/src/components/kb/csv-mapping/MappingControls.tsx),
   seeded from the auto-detected guess via `specToDraft`).

The same mapping spec shape above is sent inline; nothing about mappings is
persisted yet.

## Scope / not yet

Per-import only: there is no saved/named-mapping store, no attach-to-wallet for
recurring imports, and no AI column guessing. The engine is GUI-agnostic and the
auto-detector + `csv_inspect` already return headers + a detected mapping, so
those are additive follow-ups, not rewrites.
