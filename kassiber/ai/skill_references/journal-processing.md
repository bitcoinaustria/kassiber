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

`journals quarantined` currently has no pagination or `--limit`.

Resolve when the user has enough information:

```bash
kassiber journals quarantine resolve price-override --transaction <transaction-id> --fiat-rate <rate>
kassiber journals quarantine resolve exclude --transaction <transaction-id>
```

In chat, first read `ui.transactions.review_context` (and
`ui.transfers.review_context` for ownership or rail questions). The consented
`ui.journals.quarantine.resolve` tool is deliberately limited to reviewed price
overrides and explicit exclusions. It reprocesses by default and reports
whether the quarantine actually cleared. Never invent a rate, and never use an
exclusion to conceal a transfer or custody gap.

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

Same-asset carrying-value pairs are supported. Reviewed BTC ↔ LBTC rail changes may carry value on every profile while `bitcoin_rail_carrying_value` is enabled. Austrian policy additionally supports reviewed carrying-value treatment for other eligible crypto conversions. Cross-asset `--policy taxable` pairs stay on the normal SELL + BUY path.

Auto-detection is intentionally conservative: Kassiber only auto-pairs
rows with canonical scoped transaction identity, owned script/outpoint evidence,
or source-qualified Lightning evidence. Arbitrary provider/import ids never
establish ownership. For BTC ↔ LBTC swaps, review the surfaced pair or create
an explicit custody component when the route is incomplete.

Use `transfers components plan --action create` for 1:N, N:1, N:M,
multi-hop migrations, or missing intermediate wallets. Represent missing owned
custody with `untracked_wallet`; genuine N:M requires explicit allocations.
Activate only after exact anchor coverage and conservation pass atomically.
In chat, use `ui.transfers.components.list` to avoid duplicating reviewed state,
then `ui.transfers.components.plan` before asking
for consent to write the final component set with the returned
`expected_input_version`.

If `kassiber --machine journals transfers list` reports
`summary.cross_asset_pairs: 0`, no cross-asset swap pair is active yet.
Do not describe Austrian carry-value as already paired, already reflected in
holdings, or already visible in reports until a pair exists and journals are
reprocessed.

Timing and amount similarity can help identify candidate peg-ins / peg-outs,
but those heuristics are only for review. They do not create a pair on their
own.
