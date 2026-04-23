# Imports Reference

Kassiber can ingest transactions and metadata from several sources. Imported data lands in the local SQLite store and then participates in the normal journal and report workflow.

## Supported import paths

- generic JSON / CSV transaction files
- BTCPay CSV / JSON exports
- Phoenix CSV exports
- BIP329 JSONL labels

## Generic transaction imports

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

`amount` should be positive. If you pass a negative amount, Kassiber normalizes it and infers direction when possible.

If imported transactions do not carry `fiat_rate` or `fiat_value`, `journals process` first tries to backfill pricing from the local rates cache. Transactions only quarantine if pricing is still missing after that.

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

You can also pull confirmed on-chain wallet history directly from a BTCPay server:

```bash
python3 -m kassiber wallets sync-btcpay \
  --wallet btcpay \
  --backend btcpay-prod \
  --store-id <store-id>
```

That API-backed path reuses the same BTCPay normalization and metadata rules as the file import, but only imports confirmed rows from the remote wallet history.

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

Attachments can be added after import:

```bash
python3 -m kassiber attachments add --transaction <ID> --file /path/to/receipt.pdf
python3 -m kassiber attachments add --transaction <ID> --url https://example.com/receipt
```

File attachments are copied into Kassiber's managed attachment store. URL attachments are stored literally and are not fetched.
