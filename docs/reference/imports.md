# Imports Reference

Kassiber can ingest transactions and metadata from several sources. Imported data lands in the local SQLite store and then participates in the normal journal and report workflow.

## Supported import paths

- generic JSON / CSV transaction files
- BTCPay CSV / JSON exports
- BTCPay Greenfield confirmed wallet history
- Wasabi Wallet sanitized RPC/export bundles
- Phoenix CSV exports
- River Bitcoin Activity / Account Activity CSV exports
- Bull Bitcoin order CSV exports
- Coinfinity order CSV exports
- 21bitcoin transaction CSV exports
- Pocket Bitcoin account CSV exports
- Strike CSV exports
- Samourai Wallet recovery material for watch-only Whirlpool history
- BIP329 JSONL labels

Format references used by the dedicated importers:

- BTCPay Greenfield API: <https://docs.btcpayserver.org/Development/GreenFieldExample/>
- Wasabi RPC export methods: <https://docs.wasabiwallet.io/using-wasabi/RPC.html>
- River Account Activity CSV: <https://support.river.com/hc/en-us/articles/45513824178963-How-do-I-download-my-account-activity>
- Bull Bitcoin order CSV export from the Bull account order history
- Coinfinity order CSV export from the Coinfinity account order history
- 21bitcoin transaction CSV export from the 21bitcoin app
- Pocket Bitcoin account CSV export
- Strike CSV export from Strike transaction history
- Samourai backup and recovery docs: <https://samourai.kayako.com/section/5-samourai-backup>
- Samourai Whirlpool account docs: <https://samourai.kayako.com/article/82-understanding-deposit-premix-and-postmix-accounts>
- Samourai Whirlpool pool-fee docs: <https://samourai.kayako.com/article/81-understanding-pools-and-pool-fees>
- Samourai BIP44/BIP49/BIP84 docs: <https://samourai.kayako.com/article/65-bip-44-bip-49-and-bip84>
- Sparrow Samourai backup importer: <https://raw.githubusercontent.com/sparrowwallet/sparrow/master/src/main/java/com/sparrowwallet/sparrow/io/Samourai.java>
- Drongo Samourai crypto/account helpers:
  <https://raw.githubusercontent.com/sparrowwallet/drongo/master/src/main/java/com/sparrowwallet/drongo/crypto/SamouraiUtil.java>
  and
  <https://raw.githubusercontent.com/sparrowwallet/drongo/master/src/main/java/com/sparrowwallet/drongo/wallet/StandardAccount.java>
- Historical Sparrow Whirlpool mix-status code:
  <https://code.sparrowwallet.com/sparrowwallet/sparrow/src/commit/78f0721168f8035f418f0a01fb89ea8e942038c0/src/main/java/com/sparrowwallet/sparrow/control/MixStatusCell.java>
  and
  <https://code.sparrowwallet.com/sparrowwallet/sparrow/blame/commit/176e440195f975253cdfeab08636b1a897bf78a5/src/main/java/com/sparrowwallet/sparrow/wallet/UtxoEntry.java>
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

## Privacy-hop evidence

Privacy-aware importers may mark a transaction with the typed
`privacy_boundary` field. Supported values are `coinjoin`, `payjoin`,
`payment_in_coinjoin`, and `sweep`. Generic imports also accept source spellings
such as `privacy_hop`, `privacyHop`, `privacyBoundary`, and
`islikelycoinjoin=true`; the import boundary normalizes those spellings into the
stored `privacy_boundary` column so tax and source-funds logic do not depend on
ad hoc `raw_json` parsing.

Kassiber treats this marker as evidence of an opaque privacy boundary, not as
proof of exact upstream ownership, round membership, participant mapping, or fee
allocation. Journal normalization therefore emits `privacy_hop_unresolved` until
explicit user-owned provenance, reviewed links, or protocol-specific same-owner
recovery evidence resolves the boundary. Source-of-funds reports surface the
same marker as a warning instead of walking through unrelated participant inputs
or suggesting automatic same-transaction-id self-transfer links across it.

## Samourai Wallet and Whirlpool

`wallets import-samourai` is a local recovery importer for historical
Samourai Wallet activity. It is deliberately watch-only: Kassiber can decrypt a
local `samourai.txt` backup or consume mnemonic / descriptor material only long
enough to derive public watch descriptors, then persists the redacted wallet
configuration needed for ordinary descriptor sync. It does not connect to a
Whirlpool coordinator, does not run an active mixing client, does not create or
broadcast transactions, and does not expose seed words, passphrases, descriptors,
xpubs, PayNym secrets, backup payloads, backend URLs/tokens, or raw wallet files
through daemon results, AI tools, diagnostics, docs, or tests.

Source findings that shape the import:

- Samourai's backup docs describe an encrypted full-wallet backup restored with
  the exact wallet passphrase, and its mnemonic docs treat secret words plus the
  BIP39 passphrase as the last-resort recovery path.
- Sparrow's Samourai importer accepts the local `samourai.txt` payload, decrypts
  backup version `1` with Samourai's legacy PBKDF2/AES-CBC routine, decrypts
  version `2` with the SHA256 PBKDF2/AES-CBC routine, reads `wallet.seed`, and
  turns that seed into a normal single-sig keystore.
- Kassiber's backup import treats the backup passphrase as the Samourai wallet
  / BIP39 passphrase by default, matching standard Samourai recovery. If local
  evidence shows the backup encryption passphrase and BIP39 passphrase differ,
  provide the BIP39 value through the mnemonic-passphrase input as an override.
- Drongo's `StandardAccount` defines Whirlpool accounts as native segwit
  account roots: Badbank `2147483644'`, Premix `2147483645'`, and Postmix
  `2147483646'`; Postmix uses a minimum lookahead twice the normal default.
- Samourai's Whirlpool docs define Deposit, Premix, and Postmix as segregated
  address spaces covered by the same recovery words/passphrase. Deposit contains
  unmixed bech32 UTXOs; Premix contains UTXOs prepared by Tx0 and pending their
  first cycle; Postmix contains UTXOs that completed at least one cycle and may
  remix for free.
- Whirlpool pool docs describe Tx0 as splitting Deposit inputs into equal-sized
  Premix outputs, a flat coordinator-fee output, and toxic change. Toxic change
  belongs in Badbank / Do Not Spend review, not in the Postmix privacy set.
- Historical Sparrow Whirlpool UI treated Postmix UTXOs without stored mix data
  as at least one mix and showed exact stored mix counts when available. Kassiber
  mirrors that distinction with `minimum_mix_count=1` and separate confidence
  metadata instead of claiming exact sat lineage.
- BIP47 / PayNym roots (`m/47'/coin_type'/identity'`) are not ordinary unilateral
  receive descriptors. Kassiber records the recognized recovery root as a
  privacy limitation and only scans BIP47-derived activity when the user supplies
  explicit descriptors or already-imported transactions that prove the addresses.

Samourai source roots are interpreted as follows. Mainnet uses coin type `0'`;
testnet, signet, and regtest use coin type `1'`. The descriptor sync branches
remain normal receive/change branches (`/0/*` and `/1/*`) under each account
root.

| Section | Mainnet root | Scripts |
|---|---:|---|
| Deposit | `m/44'/0'/0'`, `m/49'/0'/0'`, `m/84'/0'/0'` | P2PKH, P2SH-P2WPKH, P2WPKH |
| Deposit PayNym | `m/47'/0'/0'` | Recognized, not scanned without explicit descriptors |
| Badbank / Toxic Change | `m/84'/0'/2147483644'` | P2WPKH |
| Premix | `m/84'/0'/2147483645'` | P2WPKH |
| Postmix | `m/84'/0'/2147483646'` | P2WPKH |
| Ricochet | `m/44'/0'/2147483647'`, `m/49'/0'/2147483647'`, `m/84'/0'/2147483647'` | P2PKH, P2SH-P2WPKH, P2WPKH |

The importer creates one logical Samourai group and child wallet sources for the
scannable sections. Child sources carry safe `samourai` metadata in their
wallet config and in UTXO provenance: section, script type, root path, pool role,
privacy boundary, minimum mix count, exact mix-count confidence when known, and
safe warning state. Descriptor and xpub material remain behind the existing
wallet redaction boundary and can only be revealed through the explicit
`wallets reveal-descriptor` owner command.
Explicit descriptor source-set imports must cover both Samourai descriptor
branches for each scanned section: branch `0` receive and branch `1` change.
Provide a separate `change_descriptor` or a descriptor expression that expands
to both branches; single-branch descriptors are rejected because they can miss
change history and understate balances, reports, and source-of-funds evidence.

Accounting behavior is intentionally conservative:

- Tx0, premix, first mix, and remix rows across the same Samourai group are
  internal same-asset privacy movement. They are not modeled as taxable disposals
  merely because public CoinJoin transactions contain unrelated participant
  inputs and outputs.
- Coordinator and miner fees remain visible on the imported/synced transaction
  rows and, when priced, through fee-only privacy events. Multi-output Tx0 rows
  are not collapsed into a fake one-to-one transfer because premix and toxic
  change are separate local destinations.
- Safe Whirlpool provenance (`pool_denomination_sat`, `target_mix_count`,
  `mix_count`, confidence, and round txids) may be carried in wallet metadata or
  UTXO `raw_json`. Kassiber drops participant graph fields and does not infer
  exact fee allocation across unrelated CoinJoin participants.
- External spends from Deposit, Postmix, Ricochet, or Badbank remain normal
  reportable rows. Toxic-change spends keep Badbank warning metadata.
- Postmix rows with no stored Whirlpool metadata are represented as "at least
  one mix" with low confidence, not as an exact round count.
- Missing prices, malformed same-asset movement, unsupported paths, or ambiguous
  privacy transitions quarantine/report blockers with actionable hints instead
  of zero-basis treatment.
- Source-of-funds reports may use `coinjoin` / `payjoin` reviewed links as
  privacy boundaries. They must not traverse, disclose, or use unrelated
  participant inputs as proof of funds.

Example CLI flows:

```bash
python3 -m kassiber wallets import-samourai \
  --label "Samourai Recovery" \
  --backup-file /path/to/samourai.txt \
  --backup-passphrase-stdin \
  --backend mempool \
  --gap-limit 80

# Add --mnemonic-passphrase-fd 3 only when the BIP39 passphrase differs from
# the backup passphrase.

python3 -m kassiber wallets import-samourai \
  --label "Samourai Recovery" \
  --mnemonic-stdin \
  --mnemonic-passphrase-fd 3 \
  --backend mempool
```

After import, run wallet sync, review any warnings or source-funds privacy
boundaries, sync/rebuild rates when pricing is missing, then run
`journals process` before trusting reports.

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
from the invoice/payment record. It also normalizes safe invoice metadata for
the desktop transaction detail view: payment-request ids, order ids, order
URLs, and origin hints such as BTCPay POS/app/external-order. Raw BTCPay
invoice JSON remains backend provenance and is not exposed to the desktop
detail panel. `suggest` creates deterministic review items from
txid/payment-hash matches and document/invoice references. Only `review`
applies authoritative `btcpay_invoice` / `btcpay_payment` pricing or a
commercial kind such as `income` to the wallet transaction; unreviewed BTCPay
file imports and wallet-history sync remain conservative `deposit` /
`withdrawal` transport rows.

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

## Wasabi Wallet

Kassiber accepts a sanitized Wasabi JSON bundle as watch-only accounting
evidence. The bundle can contain `gethistory`,
`listcoins`/`listunspentcoins`, `getwalletinfo`, `listkeys`,
`listpaymentsincoinjoin`, and optional `wallet_json` metadata. Kassiber does
not control the wallet, does not persist raw `wallet.json`, and does not expose
seeds, encrypted secrets, chain code, xpub/extpub/public keys, full key paths,
backend URLs, or raw wallet blobs through UI, AI, diagnostics, or public report
surfaces.

Import directly:

```bash
python3 -m kassiber wallets import-wasabi \
  --wallet wasabi \
  --file /path/to/wasabi-sanitized-bundle.json
```

Behavior:

- `gethistory` rows become wallet transactions using Wasabi's signed net wallet
  effect, timestamp/block height, label, txid, and `islikelycoinjoin` evidence
- `listcoins` / `listunspentcoins` refresh the durable Coins/UTXO inventory,
  including amount, confirmations, address label, safe receive/change branch
  and index, anonymity score, spent-by txid, CoinJoin exclusion, key state, and
  sanitized anonymity history when present
- `getwalletinfo`, `listkeys`, and `wallet_json` only enrich safe wallet
  metadata such as anonymity-score target, AutoCoinJoin/RedCoinIsolation,
  watch-only/hardware flags, gap limit, account-path hints, and key-state
  counts
- CoinJoin and PayJoin evidence is treated as a reviewed privacy boundary:
  Kassiber preserves tx-level flags and coin-level anonymity evidence, but does
  not fabricate round ids, participant mappings, upstream ownership, or exact
  foreign-input fees
- Journal/report readiness marks ambiguous CoinJoin, payment-in-CoinJoin,
  PayJoin, or sweep evidence as `privacy_hop_unresolved` until the user adds
  explicit user-owned provenance

You can also create a wallet whose source file is a Wasabi bundle:

```bash
python3 -m kassiber wallets create \
  --label wasabi \
  --kind wasabi \
  --source-file /path/to/wasabi-sanitized-bundle.json \
  --source-format wasabi_bundle

python3 -m kassiber wallets sync --wallet wasabi
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

## Coinfinity

Kassiber supports Coinfinity order CSV exports as exchange evidence. Coinfinity
orders often settle directly on-chain, so the import mirrors the Bull Bitcoin
flow: rows are matched book-wide against existing wallet transactions by
transaction id, asset, amount, and direction. Relevant mode enriches the real
wallet row; full mode imports excluded provider evidence and flags gaps.

Import directly:

```bash
python3 -m kassiber wallets import-coinfinity \
  --file /path/to/coinfinity-orders.csv
```

The default `--mode relevant` enriches only rows that uniquely match existing
transactions anywhere in the active profile.

`--mode full` imports every normalized Coinfinity order into the selected
wallet, or into a default `Coinfinity` wallet when no wallet is supplied. Full
mode keeps the imported provider rows excluded from accounting by default and
adds reconciliation tags:

- `coinfinity-matched` means the Coinfinity row matched one existing wallet
  transaction in this book
- `coinfinity-wallet-gap` means no matching wallet transaction was found in
  this book; the row may be a missing wallet sync or may belong to another book
- `coinfinity-ambiguous` means more than one wallet transaction matched and
  the row needs review

```bash
python3 -m kassiber wallets import-coinfinity \
  --mode full \
  --file /path/to/coinfinity-orders.csv
```

Behavior:

- Coinfinity `sell` rows are user BTC buys, because Coinfinity sold BTC to the
  user; Kassiber records them as inbound `buy` transactions
- Coinfinity `buy` rows are user BTC sells, because Coinfinity bought BTC from
  the user; Kassiber records them as outbound `sell` transactions
- `Amount EUR` plus `Total Fee EUR` is used as the exact buy cost basis
- `Amount EUR` minus `Total Fee EUR` is used as the exact sell proceeds
- `Mining Fee Crypto` is stored as the BTC fee on outbound sell rows when
  present
- relevant imports are match-existing-only and never create standalone
  transactions; full imports create excluded evidence rows

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

## Pocket Bitcoin

Kassiber supports Pocket Bitcoin account CSV exports as exchange evidence. The
Pocket export records fiat deposits, exchange executions, and BTC withdrawals
as separate rows. Kassiber imports the exchange rows, pairs the matching BTC
withdrawal row when present, and preserves the execution price as exact
`exchange_execution` pricing.

Import directly:

```bash
python3 -m kassiber wallets import-pocket \
  --file /path/to/pocket-account.csv
```

The default `--mode relevant` mirrors the Bull Bitcoin flow: it enriches only
rows that uniquely match an existing transaction in the active profile. Pocket
does not export the blockchain txid in this CSV, so matching uses the net BTC
withdrawal amount, direction, asset, and nearby timestamp instead of a txid.

`--mode full` imports every exchange row into the selected wallet, or into a
default `Pocket Bitcoin` wallet when no wallet is supplied. Full mode keeps the
imported Pocket rows excluded from accounting by default and adds
reconciliation tags:

- `pocketbitcoin-matched` means the Pocket row matched one existing wallet
  transaction in this book
- `pocketbitcoin-wallet-gap` means no matching wallet transaction was found in
  this book
- `pocketbitcoin-ambiguous` means more than one wallet transaction matched and
  the row needs review

```bash
python3 -m kassiber wallets import-pocket \
  --mode full \
  --file /path/to/pocket-account.csv
```

Behavior:

- fiat -> Bitcoin rows become inbound `buy` transactions
- Bitcoin -> fiat rows become outbound `sell` transactions when present
- fiat exchange fees are included in buy cost basis and reduce sell proceeds
- paired BTC withdrawal fees are stored on the imported Pocket evidence row
- relevant imports are match-existing-only and never create standalone
  transactions; full imports create excluded evidence rows

You can also attach a Pocket export to an existing wallet:

```bash
python3 -m kassiber wallets update --wallet treasury \
  --config '{"source_file":"/path/to/pocket-account.csv","source_format":"pocketbitcoin_csv"}'

python3 -m kassiber wallets sync --wallet treasury
```

## Strike

Kassiber supports Strike CSV exports as a custodial platform ledger for BTC
activity. Strike can be used as both an exchange and an everyday wallet, so
Kassiber imports the BTC-side platform ledger into the selected custodial
wallet, or into a default `Strike` wallet when no wallet is supplied. Fiat-only
platform funding and reversal rows are skipped because they are not Bitcoin
subledger activity.

```bash
python3 -m kassiber wallets import-strike \
  --file /path/to/strike-export.csv
```

Behavior:

- positive `Amount BTC` rows become inbound transactions
- negative `Amount BTC` rows become outbound transactions
- buy and sell rows preserve exact exchange execution pricing when the export
  includes `BTC Price` or fiat amount columns
- Lightning invoice rows use a provider-scoped id (`strike:<Reference>`) and
  preserve the exported 64-character hash as `payment_hash` when present
- on-chain rows use `Transaction Hash` as the transaction id when Strike
  provides one
- `BTC Price` is used as the exact CSV rate when present; fiat amount columns
  and buy-row cost basis can fill pricing when Strike does not export a price
- fiat-only deposit and reversal rows are ignored by the importer

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
python3 -m kassiber metadata records history list --transaction <ID>
python3 -m kassiber metadata records history activity --source ai_tool
```

Metadata edits from the CLI, desktop, and approved AI tools write append-only
history in the same local database transaction as the actual change. No-op
saves do not create history rows, and `metadata records history revert` records
an undo as a new forward edit rather than modifying the old event.

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

File attachments are copied into Kassiber's managed attachment store. URL
attachments are stored literally; the desktop transaction detail view may store
a fetched page title as the display label, but linked content is not copied or
indexed. That display label can be edited without changing the URL target.
