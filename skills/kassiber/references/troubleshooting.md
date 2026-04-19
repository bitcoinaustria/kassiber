# Troubleshooting

Use this reference when Kassiber output looks wrong, empty, or inconsistent with expectations.

## Empty reports

Most common cause:

```bash
kassiber journals process
```

If the user recently synced, imported, tagged, excluded, or changed rates, re-process journals before diagnosing anything deeper.

## Missing prices or partial tax output

Check quarantine:

```bash
kassiber journals quarantined
kassiber journals quarantine show --transaction <transaction-id>
```

Then sync rates or add a manual rate:

```bash
kassiber rates sync
kassiber rates set BTC-EUR 2025-01-01T00:00:00Z 95000
```

and process again.

## Wrong scope

Confirm where Kassiber is pointed:

```bash
kassiber status
kassiber context show
```

If needed, use explicit scope flags instead of relying on context.

## Liquid sync failures

Verify all of:

- wallet kind is `descriptor`
- `--backend` points at a Liquid-capable backend
- descriptor includes private blinding keys
- network is correct, usually `liquidv1`

## Command not found

If `kassiber` is missing from `PATH`, use:

```bash
uv run kassiber status
```

or activate the local environment:

```bash
source .venv/bin/activate
kassiber status
```

## Path confusion

Kassiber may use `~/.kassiber` or a legacy XDG location depending on existing state. Do not assume. Read:

```bash
kassiber status
```

and trust the reported `state_root`, `data_root`, and `database` fields.
