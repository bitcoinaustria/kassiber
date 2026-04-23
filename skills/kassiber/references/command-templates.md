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
  --account <account-name> \
  --backend mempool \
  --descriptor-file <receive-descriptor-file> \
  --change-descriptor-file <change-descriptor-file>
```

Liquid descriptor template:

```bash
kassiber wallets create \
  --label <wallet-label> \
  --kind descriptor \
  --account <account-name> \
  --backend liquid \
  --chain liquid \
  --network liquidv1 \
  --descriptor-file <receive-descriptor-file> \
  --change-descriptor-file <change-descriptor-file>
```

BTCPay backend + sync template:

```bash
kassiber backends create <btcpay-backend-name> \
  --kind btcpay \
  --url <btcpay-base-url> \
  --token "$BTCPAY_TOKEN"
kassiber wallets sync-btcpay \
  --wallet <wallet-label> \
  --backend <btcpay-backend-name> \
  --store-id <btcpay-store-id>
```

## Transactions

`transactions` needs the `list` subcommand:

```bash
kassiber --machine transactions list
kassiber --machine transactions list --limit 100 --cursor <cursor>
```

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
