# Kassiber

Kassiber is an open-source, local-first Bitcoin accounting CLI.

The name means "notes smuggled past prison censors." The cloud-SaaS tool is the censor: the middleman reading everything before it reaches the state. Kassiber slips past.

Before pointing Kassiber at real wallets, read [SECURITY.md](SECURITY.md) — it
covers what each built-in backend sees, the full external-request inventory,
and caveats like at-rest encryption and `tor_proxy` not being wired up yet.

It is designed around:

- multiple workspaces
- multiple profiles per workspace
- multiple accounts per profile
- multiple wallets per profile
- explicit profile tax policy defaults
- dotenv-driven and DB-backed named sync backends
- explicit journal processing before reporting
- capital gains, balance sheet, portfolio, and balance-history reporting
- a cached exchange-rate table with manual override and CoinGecko sync
- a machine-readable JSON envelope for every command

By default Kassiber keeps user state in a hidden home tree:
`~/.kassiber/data/kassiber.sqlite3` for the SQLite store,
`~/.kassiber/config/backends.env` for backend config,
`~/.kassiber/config/settings.json` for the managed state manifest, and
`~/.kassiber/exports/` as the reserved place to keep generated report files,
plus `~/.kassiber/attachments/` for managed transaction attachment blobs.
`kassiber status` shows the active paths, and `--data-root` / `--env-file`
let you override them.

## What is implemented

### Storage & model
- local SQLite-backed storage (system of record)
- `workspaces`, `profiles`, `accounts`, `wallets`
- explicit profile tax policy via `tax_country`, `tax_long_term_days`, `gains_algorithm`

### Output & control
- global `--format {table,plain,json,csv}` with `--output <path>`
- `--machine` shortcut (equivalent to `--format json`)
- structured success envelope `{kind, schema_version, data}`
- structured error envelope `{code, message, hint, details, retryable, debug}`
- `--debug` surfaces stack traces inside the error envelope

### Wallets
- kinds: `descriptor`, `xpub`, `address`, `coreln`, `lnd`, `nwc`, `phoenix`, `river`, `custom`
- `wallets kinds` — catalog of supported kinds with `requires` + `config_fields`
- full CRUD: `create`, `list`, `get`, `update` (label / account / config merge / altbestand / `--clear FIELD`), `delete [--cascade]`
- descriptor derivation via `wallets derive` for receive/change address and script export
- full Liquid watch-only normalization for descriptor wallets (confidential receive/change, explicit fee extraction, local unblinding of wallet-owned outputs)
- manual `Altbestand` provenance flag for tax-free disposals

### Backends
- dotenv seed + DB-backed overlay (`backends` table)
- `backends kinds / list / get / create / update / delete / set-default / clear-default`
- backend kinds: `esplora`, `electrum`, `bitcoinrpc`
- built-in Bitcoin Austria `mempool` + `fulcrum` backends, plus a bundled `liquid` Electrum endpoint
- named backends for on-chain sync (address-based Bitcoin, descriptor-backed Bitcoin on `esplora` / `electrum`, descriptor-backed Liquid on `esplora` / `electrum`)

### Imports
- generic JSON / CSV transaction files
- BTCPay CSV / JSON wallet exports (comment → note, labels → tags)
- Phoenix CSV wallet exports (description → note, payment type → tag, signed `amount_msat` → direction)
- BIP329 JSONL metadata import, listing, and export

### Journals & processing
- `journals process` runs the RP2 tax engine (FIFO/LIFO/HIFO/LOFO) per asset, pooling lots across wallets so cost basis follows on-chain coins between user-owned wallets
- detects on-chain self-transfers (same `txid` outbound + inbound across two wallets of the same profile) and books them as RP2 `IntraTransaction` (MOVE): the network fee is the only realized disposal, and the moved coins keep their original cost basis at the destination wallet
- emits `transfer_out`, `transfer_in`, and (when there's a fee) `transfer_fee` journal entries for each detected pair; the response envelope reports `transfers_detected`
- `transfers pair / list / unpair` lets you manually link an outbound + inbound transaction across wallets when the auto-detector can't (different `txid`s, peg-in/peg-out, submarine swaps); same-asset manual pairs currently support `--policy carrying-value` and feed the IntraTransaction path, while cross-asset pairs are recorded as audit metadata
- `journals events list` with `--wallet / --account / --asset / --entry-type / --start / --end` filters and opaque base64 cursor pagination
- `journals events get --event-id` for a single entry
- `journals quarantined` surfaces outbound transactions that couldn't be priced or lot-matched

### Metadata
- `metadata records list` with `--wallet / --tag / --has-note / --no-note / --excluded / --included / --start / --end` and cursor pagination
- `metadata records get --transaction` — unified per-transaction view (note, tags, excluded)
- `metadata records note set/clear`, `tag add/remove`, `excluded set/clear`

### Attachments
- `attachments add --transaction <tx> --file <path>` copies a local file into Kassiber's managed attachment store
- `attachments add --transaction <tx> --url <https://...>` stores a URL reference as a literal string without fetching or indexing it
- `attachments list`, `attachments remove`, `attachments verify`, and `attachments gc` manage and audit the local attachment store
- the managed attachment root is exposed in `kassiber init`, `kassiber status`, and `config/settings.json` as `attachments_root` so backups can include it with the SQLite state

### Reports
- `reports balance-sheet` — current account / liability / equity breakdown
- `reports portfolio-summary` — per-wallet holdings, cost basis, market value, unrealized PnL
- `reports capital-gains` — per-disposal realized gains/losses for tax reporting
- `reports journal-entries` — raw double-entry ledger export
- `reports balance-history --interval {hour,day,week,month}` with `--start` / `--end` and `--wallet / --account / --asset` filters
- `reports export-pdf --file report.pdf [--wallet <wallet>]` — comprehensive landscape PDF summary with complete holdings, flows, capital gains, history, transactions, and data-quality metrics

### Rates
- `rates pairs` — list supported pairs and per-pair cache coverage
- `rates sync [--pair BTC-USD] [--days N] [--source coingecko]` — pull historical spot prices from CoinGecko into the local cache
- `rates latest <PAIR>` — most recent cached sample for a pair
- `rates range <PAIR> [--start X] [--end Y] [--limit N]` — cached samples in a window, CSV-exportable
- `rates set <PAIR> <TIMESTAMP> <RATE> [--source manual]` — upsert a manual rate without hitting the network

Supported pairs today: `BTC-USD`, `BTC-EUR`. The cache is additive: a manual override and a synced rate can coexist at the same timestamp under different `source` values. The rates cache is an external data store; tax-aware reports still derive their fiat rates from priced transactions.

Cost basis is pooled per asset across all wallets in a profile so the RP2 lot engine can match a disposal in one wallet against an acquisition in another and so on-chain self-transfers (booked as `IntraTransaction` MOVE) carry their original basis to the destination wallet. Per-wallet portfolio rows show that wallet's residual quantity multiplied by the asset's average residual basis — useful as an allocation, not as an authoritative answer to "which lot lives where." SQLite remains the system of record.

## Requirements

- Python `>=3.10`
- `embit>=0.8.0`
- `rp2>=1.7.2`

The Python floor is set by the current RP2 and `embit` dependencies. RP2 is not an optional add-on: Kassiber uses it as the tax engine for journal processing and tax-aware reports.

## Installation

The recommended install path is a virtual environment:

```bash
python3 -m venv .venv
source .venv/bin/activate
python3 -m pip install --upgrade pip
python3 -m pip install -e .
```

This installs Kassiber together with RP2, `embit`, and the rest of the supported runtime dependencies.

`uv sync` is also supported and is used by the project's `uv.lock`.

## JSON envelope

Every command accepts `--machine` (or `--format json`) and emits a deterministic envelope:

```json
{
  "kind": "reports.balance-history",
  "schema_version": 1,
  "data": [
    {
      "period_start": "2025-01-01T00:00:00Z",
      "period_end": "2025-01-31T23:59:59Z",
      "asset": "BTC",
      "quantity": 0.8,
      "cumulative_cost_basis": 27000.0,
      "market_value": 36000.0
    }
  ]
}
```

Errors use the same shape with `kind: "error"`:

```json
{
  "kind": "error",
  "schema_version": 1,
  "error": {
    "code": "validation",
    "message": "Invalid start timestamp 'not-a-date'",
    "hint": "Use RFC3339 UTC like 2025-01-01T00:00:00Z",
    "details": null,
    "retryable": false,
    "debug": null
  }
}
```

`--format plain` produces key-value output for display; `--format csv` with `--output <path>` writes a file suitable for spreadsheets and works on any command that returns a list of dicts.

## Quick start

```bash
python3 -m kassiber init
python3 -m kassiber workspaces create personal
python3 -m kassiber context set --workspace personal
python3 -m kassiber profiles create main \
  --fiat-currency USD \
  --tax-country generic \
  --tax-long-term-days 365 \
  --gains-algorithm FIFO
python3 -m kassiber context set --profile main
python3 -m kassiber wallets create \
  --label coldcard \
  --kind descriptor \
  --source-file examples/sample-wallet.json \
  --source-format json
python3 -m kassiber wallets sync --wallet coldcard
python3 -m kassiber journals process
python3 -m kassiber reports balance-sheet
python3 -m kassiber reports balance-history --interval month \
  --start 2025-01-01T00:00:00Z --end 2025-12-31T23:59:59Z
python3 -m kassiber reports export-pdf --file report.pdf
```

The first `init` creates the default hidden home state tree if it does not
exist yet:

- `~/.kassiber/data`
- `~/.kassiber/config`
- `~/.kassiber/exports`
- `~/.kassiber/attachments`

`settings.json` is written into `~/.kassiber/config/` automatically and records
the active path layout (`state_root`, `data_root`, `database`, `env_file`,
`exports_root`, `attachments_root`) so the whole state tree is easy to inspect.

## Backends via `.env`

Kassiber loads named sync backends from a dotenv file. By default that path
is `~/.kassiber/config/backends.env` (or the matching `config/backends.env`
state sibling when you override `--data-root`). Without any user config it
already includes:

- `mempool` → `esplora` → `https://mempool.bitcoin-austria.at/api`
- `fulcrum` → `electrum` → `ssl://index.bitcoin-austria.at:50002`
- `liquid` → `electrum` → `ssl://les.bullbitcoin.com:995`

`mempool` remains the built-in default backend, and the bundled `liquid`
endpoint is ready for Liquid descriptor sync over Electrum.

Address-based Bitcoin wallets can use the default backend with no extra setup:

```bash
python3 -m kassiber wallets create \
  --label donations \
  --kind address \
  --address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq

python3 -m kassiber wallets sync --wallet donations
```

Mark a wallet as tax-free Altbestand:

```bash
python3 -m kassiber wallets set-altbestand --wallet donations
python3 -m kassiber wallets list
```

Switch back to normal Neubestand treatment:

```bash
python3 -m kassiber wallets set-neubestand --wallet donations
```

Inspect backends loaded from `.env` + DB:

```bash
python3 -m kassiber backends list
```

Env key pattern:

- `KASSIBER_DEFAULT_BACKEND`
- `KASSIBER_BACKEND_<NAME>_<FIELD>`

### DB-backed backend CRUD

Backends defined in `.env` are read-only. For interactive workflows, Kassiber also keeps a `backends` SQLite table that overlays the `.env` seed:

```bash
python3 -m kassiber backends create fulcrum --kind electrum \
  --url ssl://index.bitcoin-austria.at:50002 --batch-size 100 --timeout 30
python3 -m kassiber backends update fulcrum --batch-size 50 --timeout 60
python3 -m kassiber backends set-default fulcrum
python3 -m kassiber backends clear-default
python3 -m kassiber backends delete fulcrum
```

`backends set-default` stores the choice in the `settings` table and overrides whatever `KASSIBER_DEFAULT_BACKEND` was loaded from `.env`.

### Implemented backend kinds

- `esplora`
- `electrum`
- `bitcoinrpc`

### Common backend fields

- `KIND`
- `URL`
- `TIMEOUT`
- `CHAIN` — optional. Helps catch Bitcoin/Liquid backend mixups early.
- `NETWORK` — optional. Helps catch mainnet/testnet/regtest mismatches early.

### Electrum backend fields

- `URL` — example: `ssl://index.bitcoin-austria.at:50002`
- `BATCH_SIZE` — optional. Number of Electrum RPC calls Kassiber pipelines per batch. Defaults to `100`.
- `TIMEOUT`
- `INSECURE` — optional. Disables TLS certificate verification for `ssl://` backends.

Kassiber uses Electrum's scripthash API and falls back to raw transaction decoding, so it works with servers that do not expose verbose transaction JSON.

### Bitcoin Core backend fields

- `URL` — example: `http://127.0.0.1:8332`
- `USERNAME`
- `PASSWORD`
- `COOKIEFILE` — optional alternative to username/password auth
- `WALLETPREFIX` — optional. Defaults to `kassiber`.

For `bitcoinrpc`, Kassiber creates or loads a dedicated watch-only Bitcoin Core wallet per Kassiber wallet. This keeps multi-wallet sync isolated instead of mixing unrelated addresses together in one Core wallet.

### Example `.env`

Copy [.env.example](.env.example) to `~/.kassiber/config/backends.env` if you
want a real file to edit, or point Kassiber somewhere else with `--env-file`.

```dotenv
KASSIBER_DEFAULT_BACKEND=mempool

KASSIBER_BACKEND_MEMPOOL_KIND=esplora
KASSIBER_BACKEND_MEMPOOL_CHAIN=bitcoin
KASSIBER_BACKEND_MEMPOOL_NETWORK=main
KASSIBER_BACKEND_MEMPOOL_URL=https://mempool.bitcoin-austria.at/api

KASSIBER_BACKEND_FULCRUM_KIND=electrum
KASSIBER_BACKEND_FULCRUM_CHAIN=bitcoin
KASSIBER_BACKEND_FULCRUM_NETWORK=main
KASSIBER_BACKEND_FULCRUM_URL=ssl://index.bitcoin-austria.at:50002
KASSIBER_BACKEND_FULCRUM_BATCH_SIZE=100
KASSIBER_BACKEND_FULCRUM_TIMEOUT=30

KASSIBER_BACKEND_CORE_KIND=bitcoinrpc
KASSIBER_BACKEND_CORE_CHAIN=bitcoin
KASSIBER_BACKEND_CORE_NETWORK=main
KASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332
KASSIBER_BACKEND_CORE_COOKIEFILE=~/.bitcoin/.cookie
KASSIBER_BACKEND_CORE_WALLETPREFIX=kassiber

KASSIBER_BACKEND_LIQUID_KIND=electrum
KASSIBER_BACKEND_LIQUID_CHAIN=liquid
KASSIBER_BACKEND_LIQUID_NETWORK=liquidv1
KASSIBER_BACKEND_LIQUID_URL=ssl://les.bullbitcoin.com:995
KASSIBER_BACKEND_LIQUID_BATCH_SIZE=100
```

Wallets can point at a named backend with `--backend <name>`. If omitted, the current default is used.

## Descriptor wallets

Descriptor-backed wallets derive receive and change scripts locally, then sync through named backends without hardcoding a specific wallet provider.

Bitcoin example:

```bash
python3 -m kassiber wallets create \
  --label vault \
  --kind descriptor \
  --backend mempool \
  --descriptor 'wpkh([fingerprint/84h/0h/0h]xpub.../0/*)' \
  --change-descriptor 'wpkh([fingerprint/84h/0h/0h]xpub.../1/*)' \
  --gap-limit 20

python3 -m kassiber wallets derive --wallet vault --count 5
python3 -m kassiber wallets sync --wallet vault
```

Liquid example:

```bash
python3 -m kassiber wallets create \
  --label event-liquid \
  --kind descriptor \
  --backend liquid \
  --chain liquid \
  --network liquidv1 \
  --descriptor 'ct(slip77(...),elwpkh(.../0/*))' \
  --change-descriptor 'ct(slip77(...),elwpkh(.../1/*))' \
  --gap-limit 20
```

For Liquid:

- Kassiber does not ship a built-in public Liquid backend default. Point the wallet at an explicitly named backend in `~/.kassiber/config/backends.env` (or your chosen `--env-file`).
- Private blinding keys are required for full sync, balances, and fee accounting.
- Kassiber accepts modern `ct(...)` / `elwpkh(...)` Liquid descriptor syntax and normalizes it internally for the current descriptor library.

`wallets derive` is useful for matching exports against your wallet scripts, checking receive/change branches locally, or feeding custom dashboards.

In JSON output, each derived row now includes:

- `derivation_path` for single-path descriptors
- `derivation_paths` for the exact per-key BIP32 paths used at that leaf
- `key_origins` in descriptor-style `[fingerprint/path]` notation, which is especially useful for multisig

## BTCPay imports

Kassiber supports BTCPay wallet exports in CSV or JSON form.

Import a BTCPay export:

```bash
python3 -m kassiber wallets import-btcpay \
  --wallet btcpay \
  --file /path/to/btcpay-transactions.csv \
  --input-format csv
```

- transaction rows become Kassiber transactions
- `Comment` becomes the transaction note if the note is empty
- `Labels` become Kassiber tags

Or use a BTCPay file as a wallet sync source:

```bash
python3 -m kassiber wallets create \
  --label btcpay \
  --kind address \
  --altbestand \
  --address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq \
  --source-file /path/to/btcpay-transactions.csv \
  --source-format btcpay_csv
```

## Phoenix imports

Kassiber reads [Phoenix](https://phoenix.acinq.co/) wallet exports in CSV form.

```bash
python3 -m kassiber wallets import-phoenix \
  --wallet phoenix \
  --file /path/to/phoenix-export.csv
```

- Phoenix's signed `amount_msat` column drives the direction (negative → outbound)
- `mining_fee_sat * 1000 + service_fee_msat` becomes the Kassiber `fee`
- `amount_fiat` (`"22.9998 USD"`) becomes `fiat_value`; `fiat_rate` is derived from value ÷ amount
- Phoenix `description` becomes the transaction note if the note is empty
- Phoenix payment `type` (`lightning_received`, `lightning_sent`, `swap_in`, `swap_out`, `channel_close`, `liquidity_purchase`, `fee_bumping`, `legacy_swap_in`, `legacy_swap_out`, `legacy_pay_to_open`) becomes a Kassiber tag

Or use a Phoenix export as a wallet sync source:

```bash
python3 -m kassiber wallets create \
  --label phoenix \
  --kind phoenix \
  --source-file /path/to/phoenix-export.csv \
  --source-format phoenix_csv
```

## BIP329

Kassiber stores imported BIP329 records in SQLite and bridges transaction labels into Kassiber tags.

```bash
python3 -m kassiber metadata bip329 import \
  --wallet donations \
  --file /path/to/labels.jsonl

python3 -m kassiber metadata bip329 list --wallet donations

python3 -m kassiber metadata bip329 export \
  --wallet donations \
  --file /path/to/export.jsonl
```

Kassiber preserves the full BIP329 record and uses transaction labels to create tags when the referenced txid already exists locally.

## Metadata records

The `metadata records` namespace is the canonical interface for per-transaction bookkeeping: notes, tags, and inclusion/exclusion.

```bash
# List with filters + cursor pagination
python3 -m kassiber metadata records list \
  --wallet coldcard --has-note --limit 50

# Single-transaction view (note, tags, excluded)
python3 -m kassiber metadata records get --transaction <TRANSACTION_ID>

# Note CRUD
python3 -m kassiber metadata records note set --transaction <ID> --note "Cold storage move"
python3 -m kassiber metadata records note clear --transaction <ID>

# Tag CRUD (tags are created on-demand with a code)
python3 -m kassiber metadata records tag add --transaction <ID> --tag tax-lot
python3 -m kassiber metadata records tag remove --transaction <ID> --tag tax-lot

# Exclude / include from reporting
python3 -m kassiber metadata records excluded set --transaction <ID>
python3 -m kassiber metadata records excluded clear --transaction <ID>
```

## Journal events

```bash
# Paginate through processed ledger events
python3 -m kassiber journals events list \
  --asset BTC --entry-type disposal --start 2025-01-01T00:00:00Z

python3 -m kassiber journals events get --event-id <EVENT_ID>

# Quarantined transactions
python3 -m kassiber journals quarantined
```

## Transfer pairs

Cross-wallet self-transfers are auto-detected when both legs share the same on-chain `txid`. When that signal is missing — different transactions per leg, BTC ↔ LBTC peg-ins/peg-outs, Lightning ↔ on-chain submarine swaps, manual cold-wallet rotations — link the legs explicitly:

```bash
# Same-asset manual pair (e.g. an exchange withdrawal that landed in your cold wallet
# with a different external id than the deposit row). Becomes an RP2 IntraTransaction
# (MOVE) so cost basis follows the coins to the destination wallet.
python3 -m kassiber transfers pair \
  --tx-out <OUT_TRANSACTION_ID> \
  --tx-in  <IN_TRANSACTION_ID> \
  --kind manual \
  --policy carrying-value

# Cross-asset pair (BTC outbound → LBTC inbound). Currently audit-only metadata
# — `--policy taxable` is required and the legs still process as a normal SELL +
# BUY through the lot engine. Use `--kind peg-in / peg-out / submarine-swap` to
# tag the intent.
python3 -m kassiber transfers pair \
  --tx-out <BTC_OUT_ID> \
  --tx-in  <LBTC_IN_ID> \
  --kind peg-in \
  --policy taxable \
  --note "Liquid peg-in 2026-04"

python3 -m kassiber transfers list
python3 -m kassiber transfers unpair --pair-id <PAIR_ID>
```

Manual pairs override the auto-detector: if a manual pair touches a transaction that the auto-detector also matched, the manual pair wins. After any `pair / unpair`, journals are invalidated automatically — run `journals process` again before trusting reports.

Same-asset manual pairs currently require `--policy carrying-value`. If you want the two legs to remain a taxable SELL + BUY, leave them unpaired.

Cross-asset **carrying-value** swaps (basis flows BTC → LBTC with only the network fee taxable) are not yet supported and the CLI rejects them at creation time. The blocker is unified FIFO across asset boundaries; for now all cross-asset pairs are audit metadata only.

### Resolving quarantines

Each quarantined transaction is one of `missing_spot_price`, `missing_cost_basis`, or `insufficient_lots`. The `journals quarantine` subcommands are the typed resolution paths:

```bash
# Inspect a single quarantined entry (merges quarantine row + transaction state)
python3 -m kassiber journals quarantine show --transaction <TRANSACTION_ID>

# Supply missing price data (rate and value are cross-computed from amount)
python3 -m kassiber journals quarantine resolve price-override \
  --transaction <TRANSACTION_ID> --fiat-rate 50000
python3 -m kassiber journals quarantine resolve price-override \
  --transaction <TRANSACTION_ID> --fiat-value 5000

# Exclude the transaction from reporting entirely
python3 -m kassiber journals quarantine resolve exclude --transaction <TRANSACTION_ID>

# Clear the quarantine flag without changing the underlying data
# (use when you fixed the upstream row directly; the next `journals process` will
# re-quarantine if the problem persists)
python3 -m kassiber journals quarantine clear --transaction <TRANSACTION_ID>
```

All three resolution paths automatically invalidate processed journals; run `journals process` afterwards to regenerate entries.

Journals must be reprocessed after any metadata, exclusion, or transaction change before reports are trusted:

```bash
python3 -m kassiber journals process
```

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

`amount` should be positive. If you provide a negative amount, Kassiber normalizes it and infers direction if possible.

RP2 needs fiat pricing to compute tax lots. If imported or synced transactions do not include `fiat_rate` / `fiat_value`, Kassiber quarantines them during `journals process` instead of silently assigning zero-basis tax results.

## Tax policy

Profiles carry their own tax policy defaults. Kassiber currently exposes the RP2-backed `generic` policy and an explicitly experimental Austrian `at` registration on top of the shared engine seam. The Austrian path normalizes profiles to EUR and preserves the legacy 365-day field shape for `Altbestand`, but journal processing intentionally stops with an `experimental_tax_policy` error until the dedicated Austrian engine is implemented and reviewed by a Steuerberater.

```bash
python3 -m kassiber profiles create austrian \
  --fiat-currency EUR \
  --tax-country at \
  --tax-long-term-days 365 \
  --gains-algorithm FIFO

python3 -m kassiber profiles list
python3 -m kassiber profiles get --profile austrian
python3 -m kassiber journals process --profile austrian
```

Wallet-level `Altbestand` stays separate from the profile policy because it is provenance metadata about specific holdings, not a country-wide rule.

## What is not implemented yet

- wiring the rates cache into journal processing (tax-aware reports still derive rates from priced transactions)
- fiat amounts stored as REAL (cents migration pending)
- River Lightning CSV importer (Phoenix CSV is supported — see above)
- `custom` wallet kind DSL for mapping arbitrary CSV schemas
- account adjustments and per-event rate overrides
- per-profile Tor proxy configuration
- xpub-native live sync without an explicit descriptor
- descriptor-backed `bitcoinrpc` live sync
- self-hosted Liquid `elements_rpc` backend support
- BTCPay Greenfield API integration
- Lightning node adapters (CoreLN / LND / NWC are defined kinds but do not yet sync)
- remote server mode / REST API
- browser / multi-user auth
- role-based access
- Austrian journal engine, Austrian defaults beyond profile registration, and E 1kv export

## Architecture notes

- The CLI entrypoint lives in [kassiber/cli/main.py](kassiber/cli/main.py), and the remaining command handlers live in [kassiber/cli/handlers.py](kassiber/cli/handlers.py).
- Supporting modules extracted from the old monolith:
  - [kassiber/errors.py](kassiber/errors.py) — `AppError` typed exception.
  - [kassiber/time_utils.py](kassiber/time_utils.py) — timestamp parsing + RFC3339 formatting.
  - [kassiber/msat.py](kassiber/msat.py) — BTC ↔ msat conversion helpers.
  - [kassiber/util.py](kassiber/util.py) — tiny type-coercion helpers.
  - [kassiber/envelope.py](kassiber/envelope.py) — JSON envelope contract and output writers.
  - [kassiber/db.py](kassiber/db.py) — SQLite schema, data-root resolution, settings helpers.
  - [kassiber/backends.py](kassiber/backends.py) — dotenv seed + DB overlay for named sync backends.
  - [kassiber/tax_policy.py](kassiber/tax_policy.py) — profile tax-policy layer.
  - [kassiber/wallet_descriptors.py](kassiber/wallet_descriptors.py) — descriptor handling.
- SQLite remains the system of record. BTC-denominated amounts (`transactions.amount`, `transactions.fee`, `journal_entries.quantity`) are stored as INTEGER msat — the Lightning convention. Machine envelopes expose both the human-friendly BTC decimal and an exact `_msat` integer for every amount.
- RP2 is used as the per-asset lot engine, with self-transfer detection promoting matching out/in pairs to RP2 `IntraTransaction` (MOVE) so basis follows on-chain BTC across user-owned wallets.
- Wallet-level `Altbestand` remains manual provenance metadata; it is not part of the profile country policy.

## Dependency policy

Kassiber is meant to become a real accounting tool, so core accounting and tax dependencies are included intentionally when they are part of the supported runtime behavior.

- `embit` is a required dependency because descriptor derivation and Liquid wallet support depend on it.
- RP2 is a required dependency because it is the current tax engine.
- Future accounting-critical dependencies should be added openly rather than hidden behind optional extras if Kassiber cannot perform its core workflow without them.
- Third-party runtime dependencies and their licenses are tracked in [THIRD_PARTY_LICENSES.md](THIRD_PARTY_LICENSES.md).

## License

GNU Affero General Public License v3.0 only (`AGPL-3.0-only`)
