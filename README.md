# Kassiber

Kassiber is an open-source, local-first Bitcoin accounting CLI.

Kassiber means "notes smuggled past prison censors." The cloud-SaaS tool is the censor: the middleman reading everything before it reaches the state. Kassiber slips past.

It is designed around:

- multiple workspaces
- multiple profiles per workspace
- multiple accounts per profile
- multiple wallets per profile
- import-driven wallet sync from CSV/JSON
- `.env`-driven named backends
- explicit journal processing before reporting
- capital gains and balance-sheet style reporting

## What is implemented

- local SQLite-backed storage
- `init`, `status`, and `context` commands
- `workspaces`, `profiles`, `accounts`, and `wallets`
- built-in default `mempool.space` Esplora backend
- wallet imports from JSON and CSV
- wallet sync from configured import sources or live Esplora backends
- transaction listing
- metadata notes, tags, include/exclude
- journal processing with FIFO/LIFO cost basis
- quarantine of outbound transactions with insufficient lots
- reports:
  - balance sheet
  - portfolio summary
  - capital gains
  - journal entries

For the MVP, cost basis is tracked per wallet, which keeps multi-wallet balances and gains isolated and correct.

## What is not implemented yet

- live Electrum / Bitcoin Core sync
- Lightning node adapters
- remote server mode
- browser auth / multi-user auth
- role-based access
- REST API

The current release is a strong local-first MVP and a solid foundation for a fully open-source replacement.

## Quick start

```bash
cd kassiber
python3 -m kassiber init
python3 -m kassiber workspaces create personal
python3 -m kassiber profiles create main
python3 -m kassiber wallets create \
  --label coldcard \
  --kind descriptor \
  --source-file examples/sample-wallet.json \
  --source-format json
python3 -m kassiber wallets sync --wallet coldcard
python3 -m kassiber journals process
python3 -m kassiber reports balance-sheet
```

## Backends via `.env`

Kassiber loads named sync backends from `.env`. If you do nothing, it already includes:

- `mempool` -> `https://mempool.space/api`

So this works out of the box for address-based wallets:

```bash
python3 -m kassiber wallets create \
  --label donations \
  --kind address \
  --address 1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa

python3 -m kassiber wallets sync --wallet donations
```

You can inspect the loaded backends with:

```bash
python3 -m kassiber backends list
```

To add your own backend, copy `.env.example` to `.env` and edit it.

Supported env keys:

- `KASSIBER_DEFAULT_BACKEND`
- `KASSIBER_BACKEND_<NAME>_KIND`
- `KASSIBER_BACKEND_<NAME>_URL`

Legacy `SATBOOKS_*` env vars are still accepted for compatibility during the rename.

Today, implemented backend kinds are:

- `esplora`

Example:

```dotenv
KASSIBER_DEFAULT_BACKEND=mempool
KASSIBER_BACKEND_MEMPOOL_KIND=esplora
KASSIBER_BACKEND_MEMPOOL_URL=https://mempool.space/api
KASSIBER_BACKEND_SELFHOST_KIND=esplora
KASSIBER_BACKEND_SELFHOST_URL=https://mempool.your-domain.com/api
```

Wallets can point at a named backend with `--backend <name>`. If omitted, the default backend is used.

## Import format

Wallet imports accept JSON arrays or CSV files with these fields:

- `occurred_at`
- `txid` or `id`
- `direction`
- `asset`
- `amount`
- `fee`
- `fiat_rate`
- `fiat_value`
- `kind`
- `description`
- `counterparty`

`amount` should be positive. If you provide a negative amount, Kassiber will normalize it and infer direction if possible.

## Example commands

```bash
python3 -m kassiber backends list
python3 -m kassiber accounts create --code ops --label "Ops Treasury" --type asset
python3 -m kassiber wallets create --label phoenix --kind phoenix --account ops --source-file examples/sample-wallet.json --source-format json
python3 -m kassiber wallets create --label donations --kind address --backend mempool --address 1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa
python3 -m kassiber wallets import-json --wallet phoenix --file examples/sample-wallet.json
python3 -m kassiber wallets sync --wallet donations
python3 -m kassiber transactions list
python3 -m kassiber metadata tags create --code tax-lot --label "Tax Lot"
python3 -m kassiber metadata tags add --transaction <TRANSACTION_ID> --tag tax-lot
python3 -m kassiber reports capital-gains
```

## Data compatibility

The CLI now defaults to `~/.local/share/kassiber/kassiber.sqlite3`.

If you already have local data under the old Satbooks path, Kassiber will fall back to `~/.local/share/satbooks/satbooks.sqlite3` when the new default path does not exist yet.

## License

MIT
