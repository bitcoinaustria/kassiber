# Imports Reference

Kassiber can ingest transactions and metadata from several sources. Imported data lands in the local SQLite store and then participates in the normal journal and report workflow.

## Supported import paths

- generic JSON / CSV transaction files
- BTCPay CSV / JSON exports
- BTCPay Greenfield confirmed wallet history
- Phoenix CSV exports
- River Bitcoin Activity / Account Activity CSV exports
- Bull Bitcoin order CSV exports
- 21bitcoin transaction CSV exports
- BIP329 JSONL labels

Format references used by the dedicated importers:

- BTCPay Greenfield API: <https://docs.btcpayserver.org/Development/GreenFieldExample/>
- River Account Activity CSV: <https://support.river.com/hc/en-us/articles/45513824178963-How-do-I-download-my-account-activity>
- Bull Bitcoin order CSV export from the Bull account order history
- 21bitcoin transaction CSV export from the 21bitcoin app
- BIP329 labels JSONL: <https://bips.xyz/329>

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
In the desktop app, Add Connection can create that BTCPay instance from URL +
API key, discover store/payment-method ids, and then finish setup in one of two
ways. `BTCPay-only` creates Kassiber wallet sources for the selected
sync-supported payment methods, so running a BTCPay store by itself is enough
for confirmed wallet-history sync. `Existing wallets` maps those BTCPay payment
methods onto already configured settlement wallets; descriptor/file sync remains
the balance source and BTCPay is used as provenance/metadata for matching
transactions.

That API-backed wallet path reuses the same BTCPay normalization and metadata
rules as the file import, but only imports confirmed rows from the remote wallet
history and records their confirmation timestamp for later rate lookup.
`wallets sync-btcpay --wallet ... --backend ... --store-id ...` still works
too. It stores the same BTCPay config on the wallet and runs the sync
immediately, so later `wallets sync` or `wallets sync --all` calls can reuse
that wallet config.

Merchant provenance is a separate path so invoice/payment facts do not duplicate
wallet balances:

```bash
python3 -m kassiber btcpay provenance sync \
  --backend btcpay-prod \
  --store-id <store-id>

python3 -m kassiber btcpay provenance suggest
python3 -m kassiber btcpay provenance links
python3 -m kassiber btcpay provenance review \
  --link <link-id> \
  --state reviewed \
  --commercial-kind income
```

`btcpay provenance sync` stores stable invoice/payment ids, raw BTCPay payload
snapshots, transaction ids/payment hashes when present, and exact fiat facts
from the invoice/payment record. `suggest` creates deterministic review items
from txid/payment-hash matches and document/invoice references. Only `review`
applies authoritative `btcpay_invoice` / `btcpay_payment` pricing or a
commercial kind such as `income` to the wallet transaction; unreviewed
BTCPay file imports and wallet-history sync remain conservative
`deposit` / `withdrawal` transport rows.

External evidence uses the same managed attachment store as transaction
attachments:

```bash
python3 -m kassiber documents create \
  --type invoice \
  --label "Invoice 2026-001" \
  --external-ref inv-1 \
  --fiat-currency EUR \
  --fiat-value 500.00

python3 -m kassiber documents attach \
  --document <document-id> \
  --file /path/to/invoice.pdf
```

Reviewed rows can be exported for accountants without exposing unrelated wallet
history:

```bash
python3 -m kassiber reports commercial-subledger
python3 -m kassiber reports export-commercial-subledger-csv --file commercial-subledger.csv
```

To wire BTCPay as enrichment-only metadata on a wallet whose balance is already tracked through descriptor or file sync, use `wallets attach-btcpay`:

```bash
python3 -m kassiber wallets attach-btcpay \
  --wallet settlement-wallet \
  --backend btcpay-prod \
  --store-id <store-id> \
  --payment-method-id BTC-CHAIN
```

The next `wallets sync` keeps the descriptor/file source as the authoritative balance, then walks the stored BTCPay route and applies comments/labels to matching transactions. Repeated `attach-btcpay` invocations dedupe identical routes and append new (store, payment method) pairs.

Current BTCPay modeling:

- use one Kassiber wallet per real underlying wallet / store-backed balance source
- multiple BTCPay stores are fine when they point at different underlying wallets
- if multiple stores point at the same underlying wallet balance, keep them on one Kassiber wallet or holdings will be duplicated
- if the real settlement wallets are already configured, map BTCPay payment methods onto those wallets instead of creating duplicate BTCPay-backed wallets
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

## River

Kassiber supports River's Bitcoin Activity and Account Activity CSV exports.
Use Account Activity when available because it carries both BTC and cash legs.

Import directly:

```bash
python3 -m kassiber wallets import-river \
  --wallet river \
  --file /path/to/river-account-activity.csv
```

Behavior:

- BTC rows become Kassiber transactions; fiat-only cash rows are skipped
- buys and sells preserve the paired cash leg as exact `exchange_execution`
  pricing from provider `River`
- rows without a paired cash leg use River's exported Bitcoin price as an
  `fmv_provider` sample when present
- USD/EUR/etc. fees on buys are included in cost basis; fiat fees on sells
  reduce proceeds
- BTC fees become Kassiber BTC fees
- `Transaction Type` / `Tag` becomes the transaction kind and a `river:*` tag
- `Method`, `Source`, and `Destination` are preserved in the description/note
- machine output includes inserted/updated transaction summaries so desktop
  imports can show what changed

You can also create a wallet whose source file is a River export:

```bash
python3 -m kassiber wallets create \
  --label river \
  --kind river \
  --source-file /path/to/river-account-activity.csv \
  --source-format river_csv

python3 -m kassiber wallets sync --wallet river
```

## Bull Bitcoin

Kassiber supports Bull Bitcoin order CSV exports as exchange evidence. Bull
accounts are often shared across multiple books for the same organization, so
the import is book-wide and mode-driven: completed Bitcoin on-chain, Lightning,
or Liquid orders with a transaction id are normalized, then reconciled against
existing transactions in the active profile. Canceled/expired orders or rows
without a transaction id are skipped.

Import directly:

```bash
python3 -m kassiber wallets import-bull \
  --file /path/to/bull-orders.csv
```

The default `--mode relevant` enriches only rows that uniquely match existing
transactions anywhere in the active profile. It is the safest mode when the same
Bull export contains operations, fundraising, and other organization activity.

`--mode full` imports every completed Bull order into the selected wallet, or
into a default `Bull Bitcoin` wallet when no wallet is supplied. Full mode keeps
the imported Bull rows excluded from accounting by default and adds
reconciliation tags:

- `bullbitcoin-matched` means the Bull row matched one existing wallet
  transaction in this book
- `bullbitcoin-wallet-gap` means no matching wallet transaction was found in
  this book; the row may be a missing wallet sync or may belong to another book
- `bullbitcoin-ambiguous` means more than one wallet transaction matched and
  the row needs review

```bash
python3 -m kassiber wallets import-bull \
  --mode full \
  --file /path/to/bull-orders.csv
```

Behavior:

- Bitcoin/Liquid/Lightning -> fiat rows become outbound `sell` transactions
- fiat -> Bitcoin/Liquid/Lightning rows become inbound `buy` transactions
- payout/payin fiat amounts are stored as exact `exchange_execution` pricing
  from provider `Bull Bitcoin`
- relevant imports are match-existing-only and never create standalone
  transactions; full imports create excluded evidence rows
- when exactly one transaction in the active profile has the same transaction
  id, asset, amount, and direction, Kassiber enriches that row and preserves
  the wallet-derived network fee
- in relevant mode, duplicate or ambiguous matches are skipped instead of
  guessed; in full mode, ambiguous rows are imported as excluded evidence and
  tagged for review

You can also attach a Bull Bitcoin export to an existing wallet that already
receives the matching transactions from another source (for example Esplora,
Electrum, Phoenix, or a descriptor sync). Bull exports are
match-existing-only: this attachment does not create standalone transaction
rows, and matching is still profile-wide.

```bash
python3 -m kassiber wallets update --wallet treasury \
  --config '{"source_file":"/path/to/bull-orders.csv","source_format":"bullbitcoin_csv"}'

python3 -m kassiber wallets sync --wallet treasury
```

## 21bitcoin

Kassiber supports 21bitcoin transaction CSV exports as a custodial platform
ledger. BTC trade rows are active custodial balance activity, not L1 wallet
transactions. Fiat-only cash deposit rows are skipped because Kassiber remains
the BTC-side subledger. The provider row id is stored as the pricing external
reference. Trade rows use a provider-scoped transaction id (`21bitcoin:<id>`).
Withdrawal rows use `linked_transaction` as the transaction id when the CSV
provides it, so they can pair with the receiving on-chain wallet row.

Import directly:

```bash
python3 -m kassiber wallets import-21bitcoin \
  --file /path/to/21bitcoin-transactions.csv
```

The default `--mode full` imports every normalized BTC-side row into the
selected custodial wallet, or into a default `21bitcoin` wallet when no wallet
is supplied. Imported rows are included in accounting. Buy/sell rows carry the
exact execution price from the CSV. Withdrawal rows intentionally do not invent
a sell price; pair them with the receiving wallet transaction so RP2 carries
the original basis out of the custodial wallet. If only part of the 21bitcoin
balance is withdrawn, the tax engine consumes only the withdrawn lots according
to the book's accounting method and leaves the remaining custodial balance with
its original basis.

`--mode relevant` is still available when the CSV should only act as evidence
against an already-imported wallet. In relevant mode, only L1 withdrawal rows
that uniquely match existing transactions anywhere in the active profile are
enriched. Internal trade rows are skipped.

```bash
python3 -m kassiber wallets import-21bitcoin \
  --mode relevant \
  --file /path/to/21bitcoin-transactions.csv
```

Behavior:

- fiat -> BTC trade rows become inbound custodial `buy` transactions
- BTC -> fiat trade rows become outbound `sell` transactions
- BTC L1 withdrawal rows become outbound `withdrawal` transactions with BTC
  fees
- EUR fees on buy trades are included in the exact acquisition value
- fiat proceeds and costs are stored as exact `exchange_execution` pricing from
  provider `21bitcoin`
- relevant imports are match-existing-only and never create standalone
  transactions; full imports create active custodial ledger rows

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
