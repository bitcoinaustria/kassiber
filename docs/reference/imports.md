# Imports Reference

Kassiber can ingest transactions and metadata from several sources. Imported data lands in the local SQLite store and then participates in the normal journal and report workflow.

## Supported import paths

- generic JSON / CSV transaction files
- BTCPay CSV / JSON exports
- BTCPay Greenfield confirmed wallet history
- Phoenix CSV exports
- BIP329 JSONL labels

## Generic transaction imports

Generic wallet imports accept JSON arrays or CSV files with these fields:

- `occurred_at`
- `confirmed_at` (optional)
- `txid` or `id`
- `direction`
- `asset`
- `amount`
- `fee`
- `fiat_rate`
- `fiat_value`
- `pricing_source_kind`
- `pricing_provider`
- `pricing_pair`
- `pricing_timestamp`
- `pricing_granularity`
- `pricing_method`
- `pricing_external_ref`
- `pricing_quality`
- `kind`
- `description`
- `counterparty`

`amount` should be positive. If you pass a negative amount, Kassiber normalizes it and infers direction when possible.

If imported transactions carry `fiat_rate` or `fiat_value`, Kassiber stores both
the legacy numeric value and an exact decimal string plus pricing provenance.
Generic imports default to `pricing_source_kind=generic_import`; source-specific
exports may pass stronger kinds such as `exchange_execution`,
`btcpay_invoice`, or `btcpay_payment`. Stronger later pricing can replace weaker
earlier pricing for the same transaction.

If imported transactions do not carry `fiat_rate` or `fiat_value`, `journals process` first tries to backfill pricing from the local rates cache. When `confirmed_at` is present, Kassiber prices from that timestamp; otherwise it falls back to `occurred_at`. Liquid Bitcoin import spellings such as `L-BTC` are normalized to `LBTC` and use the BTC fiat rate because L-BTC is pegged one-to-one with BTC. Coarse provider samples such as daily historical fallback are stored with provenance but quarantined for pricing review instead of being silently accepted as exact FMV.

For inbound transactions, explicit earn-like `kind` values such as `income`,
`interest`, `staking`, `mining`, `airdrop`, `hardfork`, `wages`,
`lending_interest`, and `routing_income` are preserved and later promoted into
RP2 earn-like receipts during journal processing. Unlabeled inbound rows stay
conservative and process as acquisitions.

## BTCPay

Import directly into an existing wallet:

```bash
python3 -m kassiber wallets import-btcpay \
  --wallet btcpay \
  --file /path/to/btcpay-transactions.csv \
  --input-format csv
```

Behavior:

- transaction rows become Kassiber transactions
- imported rows keep conservative transport kinds (`deposit` / `withdrawal`) and do not become `income` automatically
- `Comment` becomes the transaction note if the note is empty
- `Labels` become Kassiber tags

You can also sync confirmed on-chain wallet history directly from a BTCPay store:

```bash
printf %s "$BTCPAY_TOKEN" | python3 -m kassiber backends create btcpay-prod \
  --kind btcpay \
  --url https://btcpay.example.com \
  --token-stdin

python3 -m kassiber wallets create \
  --label btcpay-shop \
  --kind custom \
  --backend btcpay-prod \
  --store-id <store-id>

python3 -m kassiber wallets sync --wallet btcpay-shop

python3 -m kassiber wallets sync-btcpay \
  --wallet btcpay-shop \
  --backend btcpay-prod \
  --store-id <store-id>
```

`--token-stdin` keeps the Greenfield API key out of shell history and the
process listing. Use `--token-fd <FD>` instead when stdin is already in use.
The argv form `--token <value>` still works for legacy scripts but warns.

That API-backed path reuses the same BTCPay normalization and metadata rules as the file import, but only imports confirmed rows from the remote wallet history and records their confirmation timestamp for later rate lookup. It does not yet import BTCPay invoice/payment fiat facts; those need the future invoice/payment provenance ingest before Kassiber can treat BTCPay as the authoritative merchant price source. `wallets sync-btcpay --wallet ... --backend ... --store-id ...` still works too. It stores the same BTCPay config on the wallet and runs the sync immediately, so later `wallets sync` or `wallets sync --all` calls can reuse that wallet config.

Current BTCPay modeling:

- use one Kassiber wallet per real underlying wallet / store-backed balance source
- multiple BTCPay stores are fine when they point at different underlying wallets
- if multiple stores point at the same underlying wallet balance, keep them on one Kassiber wallet or holdings will be duplicated
- the API-backed path is still confirmed-only and is not full invoice/payment provenance yet

You can also create a wallet whose source file is a BTCPay export:

```bash
python3 -m kassiber wallets create \
  --label btcpay \
  --kind address \
  --address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq \
  --source-file /path/to/btcpay-transactions.csv \
  --source-format btcpay_csv
```

## Phoenix

Import directly:

```bash
python3 -m kassiber wallets import-phoenix \
  --wallet phoenix \
  --file /path/to/phoenix-export.csv
```

Behavior:

- signed `amount_msat` drives direction
- `mining_fee_sat * 1000 + service_fee_msat` becomes the Kassiber fee
- `amount_fiat` becomes `fiat_value`
- `fiat_rate` is derived from value divided by amount
- `description` becomes the note if the note is empty
- Phoenix payment `type` becomes a Kassiber tag

You can also create a wallet whose source file is a Phoenix export:

```bash
python3 -m kassiber wallets create \
  --label phoenix \
  --kind phoenix \
  --source-file /path/to/phoenix-export.csv \
  --source-format phoenix_csv
```

## BIP329

Kassiber stores imported BIP329 records in SQLite and bridges transaction labels into Kassiber tags when the referenced transaction is already present locally.

```bash
python3 -m kassiber metadata bip329 import --wallet donations --file /path/to/labels.jsonl
python3 -m kassiber metadata bip329 list --wallet donations
python3 -m kassiber metadata bip329 export --wallet donations --file /path/to/export.jsonl
```

## Metadata and attachments after import

The canonical interface for per-transaction bookkeeping is `metadata records`:

```bash
python3 -m kassiber metadata records list --wallet coldcard --has-note --limit 50
python3 -m kassiber metadata records get --transaction <TRANSACTION_ID>
python3 -m kassiber metadata records note set --transaction <ID> --note "Cold storage move"
python3 -m kassiber metadata records tag add --transaction <ID> --tag tax-lot
python3 -m kassiber metadata records excluded set --transaction <ID>
```

For raw transaction ranking, sort in Kassiber before applying `--limit`:

```bash
# largest inbound and outbound rows
python3 -m kassiber --machine transactions list --direction inbound --sort amount --order desc --limit 10
python3 -m kassiber --machine transactions list --direction outbound --sort amount --order desc --limit 10
# smallest outbound rows
python3 -m kassiber --machine transactions list --direction outbound --sort amount --order asc --limit 10
```

Machine output includes `has_more` and `next_cursor` when the matching row set
continues beyond the current page.

Attachments can be added after import:

```bash
python3 -m kassiber attachments add --transaction <ID> --file /path/to/receipt.pdf
python3 -m kassiber attachments add --transaction <ID> --url https://example.com/receipt
```

File attachments are copied into Kassiber's managed attachment store. URL attachments are stored literally and are not fetched.
