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
kassiber --machine commands describe wallets sync
```

Do not append `--machine` or `--format` after the subcommand tree.
`--machine` implies `--non-interactive`: it never prompts and returns a
structured `interaction_required` error when an fd/stdin secret or one-shot
input is missing. Use `commands describe [path ...]` to inspect arguments,
read/mutation class, scope flags, cursor/dry-run support, and DB requirements.

## AI chat

Use top-level `chat` for the daemon-backed assistant:

```bash
kassiber chat "Summarise report blockers and suggest next actions."
kassiber chat
kassiber chat --allow-tool ui.journals.process "Refresh journals, then summarize blockers."
kassiber chat --yes "Sync allowed wallets and summarize what changed."
kassiber chat --stream-json "List my largest outbound transactions."
kassiber --machine chat "How many transactions are missing prices?"
printf 'Explain these journal blockers:\n%s\n' "$BLOCKERS" | kassiber chat -
kassiber chat --transcript /tmp/chat-audit.ndjson "Why is my tax summary stale?"
kassiber chat --continue "And what about the year before?"
kassiber chat --incognito "One-off question, do not store this."
kassiber --machine chats list
kassiber chats config --history on
```

`kassiber chat` is the only chat command. It mirrors the desktop Assistant and
drives daemon `ai.chat`, `ai.tool_call.consent`, and `ai.chat.cancel`.
Mutating tools prompt on a TTY in rendered mode; in the REPL, `/tools` lists
the tool catalog with consent classes, `/model` and `/provider` switch
mid-session, `/allow` pre-approves a mutating tool, `/new` clears history,
and Ctrl-C cancels the current turn. For scripts, `--allow-tool
<daemon-tool-name>` approves only that tool; `--yes` approves all mutating
tools for the chat session. `--machine` emits a single final `chat` envelope;
`--stream-json` emits the raw daemon stream records as NDJSON (both one-shot
only); `chat -` reads the one-shot prompt from stdin. Neither machine mode
ever prompts for consent; unapproved mutating tool requests are denied and
fed back to the model. On a TTY, markdown renders with ANSI styling and tool
results draw deterministic tables from the daemon envelope (`--plain`
disables both). With piped stdout the raw answer is the only thing on
stdout (chrome moves to stderr), so `kassiber chat "..." > answer.txt` is
clean. Use `--no-tools` for a provider-only exchange without the tool loop,
and `--transcript <path>` to keep a plaintext NDJSON audit log of the
session's daemon records.

## Fast paths

Common requests should not require exploratory commands:

```bash
# sync every wallet in the current scope
kassiber --machine wallets sync --all

# current balances by account/bucket/asset/wallet
kassiber --format plain reports balance-sheet

# stale report repair
kassiber --machine journals process

# readiness and deterministic next-step suggestions
kassiber --machine health
kassiber --machine next-actions
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

Reconcile addresses / txids against wallets (which are mine? payment vs transfer):

```bash
kassiber wallets identify --address bc1q... --txid <64-hex>
kassiber wallets identify --file ./reconcile.txt   # one address/txid per line
kassiber wallets identify --csv ./export.csv       # smart-import any CSV shape
kassiber --format csv wallets identify --file ./reconcile.txt --output owned.csv
kassiber wallets identify --txid <64-hex> --verify-on-chain --verify-backend mempool
```

At least one of `--address` / `--txid` / `--candidate` / `--file` is required.
`--verify-on-chain` is the only path that contacts a backend; without it, txids
not already synced/imported come back `unknown`.

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

The DB passphrase has no argv form by design. Explicit fd input wins, followed
by an explicitly enrolled remembered copy, then the interactive prompt:

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
kassiber secrets remember-unlock                   # verify + enroll native store
kassiber secrets remember-unlock --passphrase-fd 3 3< /tmp/pass
kassiber secrets forget-unlock                     # revoke + clear CLI opt-in
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
kassiber rates rebuild --source coinbase-exchange --reprice-transactions
kassiber rates sync --source kraken-csv --path ~/Downloads/Kraken_OHLCVT.zip --pair BTC/EUR
kassiber rates sync --source kraken-csv --path ~/Downloads/master_q4 --pair BTC/EUR
kassiber rates set BTC-EUR 2025-01-01T00:00:00Z 95000
```

`rates range --start/--end` expects RFC3339 UTC strings, not Unix epoch values.
Use `rates rebuild --source coinbase-exchange --reprice-transactions` when
provider-derived cached prices should be discarded and rebuilt from fresh
Coinbase one-minute samples.

## Reports

```bash
kassiber --machine reports summary
kassiber --format plain reports balance-sheet
kassiber --machine reports portfolio-summary
kassiber --machine reports tax-summary
```
