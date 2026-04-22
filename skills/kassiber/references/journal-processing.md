# Journal Processing

Use this reference when the user wants tax calculations, journal entries, quarantine review, or transfer pairing.

## Processing order

Standard sequence:

```bash
kassiber wallets sync --wallet <wallet>
kassiber rates sync
kassiber journals process
```

If the wallet activity includes BTC ↔ LBTC peg-ins / peg-outs or
submarine swaps, inspect for likely outbound / inbound pairs and pair
them before `journals process`. Reports do not discover those pairs on
their own.

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
kassiber journals transfers list
kassiber transfers list
kassiber transfers pair --tx-out <txid-or-external-id> --tx-in <txid-or-external-id> --kind manual --policy carrying-value
kassiber transfers unpair --pair-id <pair-id>
```

Use `journals transfers list` to inspect the current computed transfer audit directly. It surfaces same-asset transfer matches with exact sent / received / fee amounts, plus any stored cross-asset pair links, so you do not need to infer pairing from `journals process` counts or from journal rows.

Same-asset carrying-value pairs are supported. Cross-asset `--policy carrying-value` pairs are supported for Austrian profiles and feed the swap-basis-carry path; cross-asset `--policy taxable` pairs stay on the normal SELL + BUY path.

Auto-detection is intentionally conservative: Kassiber only auto-pairs
same-asset cross-wallet transfers that share the same `external_id`.
For BTC ↔ LBTC swaps, the operator or AI helper must identify the pair
and call `kassiber transfers pair` explicitly.
