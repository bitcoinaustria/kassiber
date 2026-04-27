# Command Templates

Use this reference when a Kassiber command shape is easy to get wrong.

If a command fails with `unrecognized arguments`, stop and use one of these
templates or `--help` instead of guessing.

## Global flags

Global flags belong before the subcommand tree:

```bash
kassiber --machine status
kassiber --format plain reports balance-sheet
kassiber --format csv --output capital-gains.csv reports capital-gains
```

Do not append `--machine` or `--format` after the subcommand tree.

## Fast paths

Common requests should not require exploratory commands:

```bash
# sync every wallet in the current scope
kassiber --machine wallets sync --all

# current balances by account/bucket/asset/wallet
kassiber --format plain reports balance-sheet

# stale report repair
kassiber --machine journals process
```

## Backends

```bash
kassiber --machine backends list
kassiber backends get liquid
kassiber --machine backends set-default mempool
```

## Wallets

Create a descriptor wallet from files:

```bash
kassiber --machine wallets create \
  --label vault \
  --kind descriptor \
  --account treasury \
  --backend mempool \
  --descriptor-file /path/to/receive.desc \
  --change-descriptor-file /path/to/change.desc
```

Sync by flag, not by positional wallet id:

```bash
kassiber wallets sync --wallet vault
kassiber wallets sync --all
```

Durable wallet mutations:

```bash
kassiber wallets update --wallet vault --gap-limit 200
kassiber wallets update --wallet vault --backend fulcrum
```

`wallets update` persists config changes. Confirm with the user before using it
as a workaround unless they already asked for that mutation.

For new secret-bearing connections, prefer handing the user a local fill-in
template instead of collecting secrets in chat. Assume mainnet unless the user
explicitly says otherwise.

Bitcoin descriptor template:

```bash
kassiber wallets create \
  --label <wallet-label> \
  --kind descriptor \
  --account <bucket-code> \
  --backend mempool \
  --descriptor-file <receive-descriptor-file> \
  --change-descriptor-file <change-descriptor-file>
```

Liquid descriptor template:

```bash
kassiber wallets create \
  --label <wallet-label> \
  --kind descriptor \
  --account <bucket-code> \
  --backend liquid \
  --chain liquid \
  --network liquidv1 \
  --descriptor-file <receive-descriptor-file> \
  --change-descriptor-file <change-descriptor-file>
```

BTCPay backend + sync template:

```bash
printf %s "$BTCPAY_TOKEN" | kassiber backends create <btcpay-backend-name> \
  --kind btcpay \
  --url <btcpay-base-url> \
  --token-stdin
kassiber wallets sync-btcpay \
  --wallet <wallet-label> \
  --backend <btcpay-backend-name> \
  --store-id <btcpay-store-id>
```

The `--token-stdin` form keeps the secret out of shell history and the
process listing. Use `--token-fd <FD>` instead when stdin is already in use.

## Secret-bearing flags

Every secret-bearing CLI value has matching `--<name>-stdin` and
`--<name>-fd <FD>` variants. The argv form (e.g. `--token <value>`) still
works for legacy scripts but emits a deprecation warning and leaks to shell
history.

Covered fields: `--token`, `--password`, `--username`, `--auth-header`,
`--descriptor`, `--change-descriptor`. Constraints:

- Only one `--*-stdin` option may be active per invocation.
- Multiple `--*-fd` options may coexist; each fd is closed after the value
  is read.
- Empty values and NUL bytes are rejected up front.

Examples:

```bash
# token from a piped local variable
printf %s "$BTCPAY_TOKEN" | kassiber backends create prod \
  --kind btcpay --url https://btcpay.example.com --token-stdin

# descriptor pair from two file descriptors
kassiber wallets create \
  --label vault --kind descriptor --account treasury --backend mempool \
  --descriptor-fd 3 --change-descriptor-fd 4 \
  3< /path/to/receive.desc 4< /path/to/change.desc
```

## SQLCipher passphrase

The DB passphrase has no argv form by design. Three channels:

```bash
# interactive (controlling TTY)
kassiber status

# fd-based (parent shell opens fd 3)
kassiber --db-passphrase-fd 3 status 3< /tmp/pass

# pipe (single secret on stdin, no other --*-stdin in play)
gopass show kassiber/db-pass | kassiber --db-passphrase-fd 0 status
```

Wrong passphrase → `unlock_failed`. Missing passphrase against an encrypted
DB → `passphrase_required`. Plaintext DB → no prompt.

## Secrets

```bash
kassiber secrets status
kassiber secrets init                              # interactive prompt + confirm
kassiber secrets init --new-passphrase-fd 4 4< /tmp/new
kassiber secrets verify                            # confirm encrypted DB opens
kassiber secrets change-passphrase                 # interactive
kassiber secrets change-passphrase --db-passphrase-fd 3 --new-passphrase-fd 4 \
  3< /tmp/old 4< /tmp/new
kassiber secrets init-resume                       # inspect a half-finished migration

# lift token/password/auth_header/username out of the plaintext dotenv
# into the encrypted backends table
kassiber secrets migrate-credentials --dry-run
kassiber secrets migrate-credentials
kassiber secrets migrate-credentials --db-passphrase-fd 3 3< /tmp/pass
```

## Backups

```bash
kassiber backup export --file /tmp/snap.kassiber                   # interactive outer passphrase
kassiber backup export --file /tmp/snap.kassiber --backup-passphrase-fd 4 4< /tmp/outer
kassiber backup export --file /tmp/snap.kassiber --recipient "age1..."   # recipient mode

kassiber backup import /tmp/snap.kassiber --backup-passphrase-fd 4 4< /tmp/outer
kassiber backup import /tmp/snap.kassiber --backup-passphrase-fd 4 \
  --install --target-data-root ~/.kassiber/data 4< /tmp/outer
```

`--install` snapshots any pre-existing live data into a sibling
`pre-restore-<timestamp>/` directory before overwriting. The inner DB inside
the bundle is still SQLCipher-encrypted under the original DB passphrase.

## Reveal

```bash
kassiber backends reveal-token <name>
kassiber wallets reveal-descriptor <wallet-label> --workspace personal --profile main
```

Each request triggers a daemon `auth_required` round-trip; a wrong
passphrase produces `local_auth_denied`. Do not pipe reveal output into chat.

## Transactions

`transactions` needs the `list` subcommand:

```bash
kassiber --machine transactions list
kassiber --machine transactions list --limit 100 --cursor <cursor>
kassiber --machine transactions list --direction inbound --sort amount --order desc --limit 10
kassiber --machine transactions list --direction outbound --sort amount --order desc --limit 10
kassiber --machine transactions list --direction inbound --sort amount --order asc --limit 10
kassiber --machine transactions list --direction outbound --sort amount --order asc --limit 10
```

Use `--order desc` for largest rows and `--order asc` for smallest rows.
Do not fetch the default recent page and sort it client-side.

## Journals

```bash
kassiber journals process
kassiber journals quarantined
kassiber journals quarantine show --transaction <transaction-id>
kassiber --machine journals transfers list
```

`journals quarantined` has no `--limit`.

## Rates

```bash
kassiber rates pairs
kassiber rates latest BTC-EUR
kassiber rates range BTC-EUR --start 2025-01-01T00:00:00Z --end 2025-01-31T23:59:59Z
kassiber rates sync --pair BTC-EUR --days 30
kassiber rates set BTC-EUR 2025-01-01T00:00:00Z 95000
```

`rates range --start/--end` expects RFC3339 UTC strings, not Unix epoch values.

## Reports

```bash
kassiber --machine reports summary
kassiber --format plain reports balance-sheet
kassiber --machine reports portfolio-summary
kassiber --machine reports tax-summary
```
