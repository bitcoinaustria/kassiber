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

Do not infer historical coverage from the `samples` count in `rates sync`
output. Use `kassiber rates range BTC-EUR --start <rfc3339> --end <rfc3339>`
to verify whether the missing transaction timestamps are actually covered.

## Unrecognized arguments

If Kassiber says `unrecognized arguments`, stop and check help before trying
another guess:

```bash
kassiber --help
kassiber <command> --help
kassiber <command> <subcommand> --help
```

Common traps:

- `wallets sync` needs `--wallet <label-or-id>` or `--all`
- `transactions` needs the `list` subcommand
- `journals quarantined` has no `--limit`
- `rates range --start/--end` expects RFC3339 UTC strings
- global flags such as `--machine` and `--format` belong before the subcommand tree

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

If the user already supplied a secret-bearing Liquid descriptor, do not ask
them to paste the blinding key again just because the sync failed.

## Swap confusion

If reports show no LBTC but the wallet has Liquid transactions:

```bash
kassiber journals quarantined
kassiber --machine journals transfers list
```

If `cross_asset_pairs` is `0`, no BTC ↔ LBTC swap pair is active yet. Reports
will not show carry-value treatment until the pair exists and journals are
reprocessed.

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

If you are using the repo-local Kassiber skill bundle, remember that bundled
references live under `<skill-dir>/references/`, not repo-root `references/`.
