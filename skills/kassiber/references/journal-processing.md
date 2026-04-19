# Journal Processing

Use this reference when the user wants tax calculations, journal entries, quarantine review, or transfer pairing.

## Processing order

Standard sequence:

```bash
kassiber wallets sync --wallet <wallet>
kassiber rates sync
kassiber journals process
```

Re-run `kassiber journals process` after:

- imports
- wallet sync
- transfer pairing or unpairing
- exclusion changes
- note or tag changes that affect review flow
- rate overrides

## Process journals

```bash
kassiber journals process
kassiber journals list
```

Use explicit scope flags if needed:

```bash
kassiber journals process --workspace project-satoshi --profile main
```

## Journal events

Inspect entries:

```bash
kassiber journals events list --limit 50
kassiber journals events list --wallet satoshi-liquid --asset BTC --entry-type disposal
kassiber journals events get --event-id <event-id>
```

`journals events list` supports:

- `--wallet`
- `--account`
- `--asset`
- `--entry-type`
- `--start`
- `--end`
- `--cursor`
- `--limit`

When scripting, use `--machine` and follow `next_cursor`.

## Quarantine

List unresolved problems:

```bash
kassiber journals quarantined
kassiber journals quarantine show --transaction <transaction-id>
```

Resolve when the user has enough information:

```bash
kassiber journals quarantine resolve price-override --transaction <transaction-id> --fiat-rate <rate>
kassiber journals quarantine resolve exclude --transaction <transaction-id>
```

Clear quarantine state only when the workflow truly calls for it:

```bash
kassiber journals quarantine clear --transaction <transaction-id>
```

## Transfers

Manual transfer pairing is available when auto-detection misses a self-transfer:

```bash
kassiber transfers list
kassiber transfers pair --tx-out <txid-or-external-id> --tx-in <txid-or-external-id> --kind manual --policy carrying-value
kassiber transfers unpair --pair-id <pair-id>
```

Same-asset carrying-value pairs are supported. Cross-asset pairs are audit metadata only unless the CLI gains more explicit support.
