# Kassiber

Kassiber is an open-source, local-first Bitcoin accounting CLI.

Kassiber means "notes smuggled past prison censors." The cloud-SaaS tool is the censor: the middleman reading everything before it reaches the state. Kassiber slips past.

It is designed around:

- multiple workspaces
- multiple profiles per workspace
- multiple accounts per profile
- multiple wallets per profile
- explicit profile tax policy defaults
- `.env`-driven named backends
- explicit journal processing before reporting
- capital gains and balance-sheet style reporting

## What is implemented

- local SQLite-backed storage
- `init`, `status`, and `context` commands
- `workspaces`, `profiles`, `accounts`, and `wallets`
- explicit profile tax-policy defaults via `tax_country` and `tax_long_term_days`
- file imports from:
  - generic JSON / CSV transaction files
  - BTCPay CSV / JSON wallet exports
- live address-based sync from:
  - `esplora`
  - `electrum`
  - `bitcoinrpc`
- built-in default `mempool.space` Esplora backend
- BIP329 JSONL metadata import, listing, and export
- BTCPay label/comment bridging into Kassiber tags and notes
- transaction listing
- metadata notes, tags, include/exclude
- journal processing with RP2-backed FIFO/LIFO/HIFO/LOFO cost basis
- quarantine of outbound transactions with insufficient lots
- reports:
  - balance sheet
  - portfolio summary
  - capital gains
  - journal entries

For the current MVP, cost basis is tracked per wallet, which keeps multi-wallet balances and gains isolated and predictable. Kassiber now uses RP2 for wallet-level lot matching and cost-basis computation, while keeping SQLite as the system of record.

Wallets can also be marked manually as `Altbestand`. Disposals from an Altbestand wallet are treated as tax-free, while Neubestand wallets use normal tax treatment.

## Requirements

- Python `>=3.10`
- `rp2>=1.7.2`

The Python floor is set by the current RP2 dependency. RP2 is not an optional add-on in the current architecture: Kassiber uses it as the tax engine for journal processing and tax-aware reports.

## Installation

The recommended install path is a virtual environment:

```bash
cd /Users/dev/Github/kassiber
python3 -m venv .venv
source .venv/bin/activate
python3 -m pip install --upgrade pip
python3 -m pip install -e .
```

This installs Kassiber together with RP2 and its runtime dependencies.

If you are packaging Kassiber for broader internal use, treat RP2 as part of Kassiber's supported runtime stack rather than as an optional plugin users must discover and install separately.

## Architecture notes

- The CLI entrypoint and journal/report orchestration live in `kassiber/app.py`.
- The profile tax-policy layer lives in `kassiber/tax_policy.py`.
- SQLite remains the system of record.
- RP2 is used as the wallet-scoped lot engine.
- Wallet-level `Altbestand` remains manual provenance metadata and is not part of the profile country policy.

## Dependency policy

Kassiber is intended to become a real accounting tool, so core accounting and tax dependencies should be included intentionally when they are part of the supported runtime behavior.

- RP2 is a required dependency because it is the current tax engine.
- Future accounting-critical dependencies should be added openly rather than hidden behind optional extras if Kassiber cannot perform its core workflow without them.
- Third-party runtime dependencies and their licenses should be tracked in the repository so packaging and internal distribution stay auditable.

Current third-party license notes are tracked in [THIRD_PARTY_LICENSES.md](/Users/dev/Github/kassiber/THIRD_PARTY_LICENSES.md).

## What is not implemented yet

- descriptor/xpub derivation-backed live sync
- BTCPay Greenfield API integration
- Lightning node adapters
- remote server mode
- browser auth / multi-user auth
- role-based access
- REST API

## Quick start

```bash
cd /Users/dev/Github/kassiber
python3 -m kassiber init
python3 -m kassiber workspaces create personal
python3 -m kassiber profiles create main \
  --tax-country generic \
  --tax-long-term-days 365 \
  --gains-algorithm FIFO
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

- `mempool` -> `esplora` -> `https://mempool.space/api`

Address-based wallets can use the default backend with no extra setup:

```bash
python3 -m kassiber wallets create \
  --label donations \
  --kind address \
  --address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq

python3 -m kassiber wallets sync --wallet donations
```

Mark a wallet manually as tax-free Altbestand:

```bash
python3 -m kassiber wallets set-altbestand --wallet donations
python3 -m kassiber wallets list
```

Switch it back to normal Neubestand treatment:

```bash
python3 -m kassiber wallets set-neubestand --wallet donations
```

Inspect loaded backends with:

```bash
python3 -m kassiber backends list
```

Supported env keys follow this pattern:

- `KASSIBER_DEFAULT_BACKEND`
- `KASSIBER_BACKEND_<NAME>_<FIELD>`

Legacy `SATBOOKS_*` env vars are still accepted for compatibility during the rename.

### Implemented backend kinds

- `esplora`
- `electrum`
- `bitcoinrpc`

### Common backend fields

- `KIND`
- `URL`
- `TIMEOUT`

### Electrum backend fields

- `URL`
  Example: `ssl://electrum.blockstream.info:50002`
- `TIMEOUT`
- `INSECURE`
  Optional. Disables TLS certificate verification for `ssl://` backends.

Kassiber uses Electrum's scripthash API and falls back to raw transaction decoding, so it works with servers that do not expose verbose transaction JSON.

### Bitcoin Core backend fields

- `URL`
  Example: `http://127.0.0.1:8332`
- `USERNAME`
- `PASSWORD`
- `COOKIEFILE`
  Optional alternative to username/password auth.
- `WALLETPREFIX`
  Optional. Defaults to `kassiber`.

For `bitcoinrpc`, Kassiber creates or loads a dedicated watch-only Bitcoin Core wallet per Kassiber wallet by default. That keeps multi-wallet sync isolated instead of mixing unrelated addresses together in one Core wallet.

### Example `.env`

```dotenv
KASSIBER_DEFAULT_BACKEND=mempool

KASSIBER_BACKEND_MEMPOOL_KIND=esplora
KASSIBER_BACKEND_MEMPOOL_URL=https://mempool.space/api

KASSIBER_BACKEND_BLOCKSTREAM_KIND=electrum
KASSIBER_BACKEND_BLOCKSTREAM_URL=ssl://electrum.blockstream.info:50002
KASSIBER_BACKEND_BLOCKSTREAM_TIMEOUT=30

KASSIBER_BACKEND_CORE_KIND=bitcoinrpc
KASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332
KASSIBER_BACKEND_CORE_COOKIEFILE=~/.bitcoin/.cookie
KASSIBER_BACKEND_CORE_WALLETPREFIX=kassiber
```

Wallets can point at a named backend with `--backend <name>`. If omitted, the default backend is used.

## BTCPay imports

Kassiber supports BTCPay wallet exports in CSV or JSON form.

Import a BTCPay export with:

```bash
python3 -m kassiber wallets import-btcpay \
  --wallet btcpay \
  --file /path/to/btcpay-transactions.csv \
  --format csv
```

When importing BTCPay exports:

- transaction rows become Kassiber transactions
- `Comment` becomes the transaction note if the note is empty
- `Labels` become Kassiber tags

You can also use BTCPay files as a wallet sync source:

```bash
python3 -m kassiber wallets create \
  --label btcpay \
  --kind address \
  --altbestand \
  --address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq \
  --source-file /path/to/btcpay-transactions.csv \
  --source-format btcpay_csv
```

## BIP329

Kassiber stores imported BIP329 records in SQLite and bridges transaction labels into Kassiber tags.

Import labels:

```bash
python3 -m kassiber metadata bip329 import \
  --wallet donations \
  --file /path/to/labels.jsonl
```

List imported BIP329 records:

```bash
python3 -m kassiber metadata bip329 list --wallet donations
```

Export stored BIP329 records:

```bash
python3 -m kassiber metadata bip329 export \
  --wallet donations \
  --file /path/to/export.jsonl
```

Kassiber currently preserves the full BIP329 record and uses transaction labels to create tags when the referenced txid already exists locally.

## Import format

Generic wallet imports accept JSON arrays or CSV files with these fields:

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

RP2 needs fiat pricing to compute tax lots. If imported or synced transactions do not include `fiat_rate` / `fiat_value`, Kassiber will quarantine them during `journals process` instead of silently assigning zero-basis tax results.

## Tax policy

Profiles carry their own tax policy defaults. Today Kassiber exposes the RP2-backed `generic` policy and stores it explicitly on the profile, so adding a future country-specific RP2 policy is a small policy-layer change instead of another app-wide refactor.

```bash
python3 -m kassiber profiles create austrian \
  --fiat-currency EUR \
  --tax-country generic \
  --tax-long-term-days 365 \
  --gains-algorithm FIFO

python3 -m kassiber profiles list
```

Wallet-level `Altbestand` stays separate from the profile policy because it is provenance metadata about specific holdings, not a country-wide rule.

Existing databases are upgraded lazily on open: if an older `profiles` table is missing `tax_country` or `tax_long_term_days`, Kassiber adds them automatically with the current defaults.

## Example commands

```bash
python3 -m kassiber backends list
python3 -m kassiber accounts create --code ops --label "Ops Treasury" --type asset
python3 -m kassiber wallets create --label phoenix --kind phoenix --account ops --source-file examples/sample-wallet.json --source-format json
python3 -m kassiber wallets create --label donations --kind address --backend mempool --address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq
python3 -m kassiber wallets set-altbestand --wallet donations
python3 -m kassiber wallets sync --wallet donations
python3 -m kassiber transactions list
python3 -m kassiber metadata tags create --code tax-lot --label "Tax Lot"
python3 -m kassiber metadata tags add --transaction <TRANSACTION_ID> --tag tax-lot
python3 -m kassiber metadata bip329 import --wallet donations --file /path/to/labels.jsonl
python3 -m kassiber journals process
python3 -m kassiber reports capital-gains
```

## Data compatibility

The CLI now defaults to `~/.local/share/kassiber/kassiber.sqlite3`.

If you already have local data under the old Satbooks path, Kassiber will fall back to `~/.local/share/satbooks/satbooks.sqlite3` when the new default path does not exist yet.

## License

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`)
