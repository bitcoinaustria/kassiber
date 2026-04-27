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

## Passphrase / encrypted DB errors

If a command returns:

- `passphrase_required` — the on-disk database is SQLCipher-encrypted but no
  passphrase was supplied. Re-run interactively, or pass
  `--db-passphrase-fd <FD>` from a parent process. There is no
  `--db-passphrase <value>` flag.
- `unlock_failed` — the passphrase did not match. Double-check it; if rotated
  recently, the old passphrase no longer works.
- `plaintext_database` from `kassiber secrets change-passphrase` — the file is
  still plaintext. Run `kassiber secrets init` first.
- `already_encrypted` from `kassiber secrets init` — the file is already
  SQLCipher. Use `kassiber secrets change-passphrase` to rotate.
- `migration_leaks_plaintext` from `kassiber secrets init` — stop and inspect.
  The encrypted output appears to contain plaintext credential markers; do
  not delete the `.pre-encryption.sqlite3.bak` rollback file.
- `local_auth_denied` from a daemon reveal request — the supplied passphrase
  did not verify against the on-disk DB. Re-prompt the user; do not retry
  silently.
- `age_unavailable` from `kassiber backup ...` — neither the `age`/`rage`
  binary nor the `pyrage` Python module is available. Install one of them.
- `age_passphrase_mode_unsupported` — should not happen for default backup
  flows after V4.1; if it does, the host has `age` on `PATH` but no `pyrage`,
  and the caller forced a binary backend. Install `pyrage`.

Never embed a passphrase in argv; there is no `--db-passphrase <value>` flag
and the daemon does not accept passphrases via the request payload outside of
the `auth_response` round-trip.

If `kassiber secrets init` was interrupted, run `kassiber secrets init-resume`
to inspect the leftover `*.encrypted.sqlite3` file and decide whether to
finalize or discard it. Do not blindly rename the temp file into place.

## Plaintext-secret warning at startup

If every Kassiber command starts printing

> warning: encrypted database is in use but the bootstrap dotenv (...)
> still contains plaintext secret entries (...)

then `backends.env` carries one or more of `*_TOKEN`, `*_PASSWORD`,
`*_AUTH_HEADER`, `*_USERNAME`, `*_RPCPASSWORD`, or `*_RPCUSER` while the
database is SQLCipher-encrypted. Lift them into the encrypted DB:

```bash
kassiber secrets migrate-credentials --dry-run     # preview what would move
kassiber secrets migrate-credentials               # actually move + sanitize file
```

A `.pre-credentials-migration-<ts>.bak` of the original dotenv is saved
alongside the file. URLs, `KIND`, chain, network, and other non-secret
rows survive the rewrite untouched. The warning stops once the dotenv
no longer contains secret-shaped entries.
