# Imports Reference

Kassiber can ingest transactions and metadata from several sources. Imported data lands in the local SQLite store and then participates in the normal journal and report workflow.

To add a provider that is **not** in this list, start from
[docs/exchanges/TEMPLATE.md](../exchanges/TEMPLATE.md) and the worked examples
under [docs/exchanges/](../exchanges/README.md). Capture the provider's
custodial model, row types, and export format in a spec before any importer is
written.

## Supported import paths

- generic JSON / CSV transaction files
- generic ledger: a fill-in Excel (`.xlsx`) or CSV/TSV template for manual entry
- local-AI photo/PDF OCR drafts (`wallets preview-document` / `wallets import-document`)
- BTCPay CSV / JSON exports
- BTCPay Greenfield confirmed wallet history
- Wasabi Wallet sanitized RPC/export bundles
- Phoenix CSV exports
- River Bitcoin Activity / Account Activity CSV exports
- Bull Bitcoin order CSV exports
- Bull Bitcoin unified wallet transaction CSV exports
- Coinfinity order CSV exports
- 21bitcoin transaction CSV exports
- Pocket Bitcoin account CSV exports
- Strike CSV exports
- Samourai/Whirlpool descriptor and account-xpub source sets
- Silent Payments watch-only receiving sources (`silent-payment` wallets)
- BIP329 JSONL labels

Format references used by the dedicated importers:

- BTCPay Greenfield API: <https://docs.btcpayserver.org/Development/GreenFieldExample/>
- Wasabi RPC export methods: <https://docs.wasabiwallet.io/using-wasabi/RPC.html>
- River Account Activity CSV: <https://support.river.com/hc/en-us/articles/45513824178963-How-do-I-download-my-account-activity>
- Bull Bitcoin order CSV export from the Bull account order history
- Bull Bitcoin wallet CSV export from the Bull mobile wallet transaction-history export
- Coinfinity order CSV export from the Coinfinity account order history
- 21bitcoin transaction CSV export from the 21bitcoin app
- Pocket Bitcoin account CSV export
- Strike CSV export from Strike transaction history
- Samourai Whirlpool account docs: <https://samourai.kayako.com/article/82-understanding-deposit-premix-and-postmix-accounts>
- Samourai Whirlpool pool-fee docs: <https://samourai.kayako.com/article/81-understanding-pools-and-pool-fees>
- Samourai BIP44/BIP49/BIP84 docs: <https://samourai.kayako.com/article/65-bip-44-bip-49-and-bip84>
- Drongo Samourai crypto/account helpers:
  <https://raw.githubusercontent.com/sparrowwallet/drongo/master/src/main/java/com/sparrowwallet/drongo/crypto/SamouraiUtil.java>
  and
  <https://raw.githubusercontent.com/sparrowwallet/drongo/master/src/main/java/com/sparrowwallet/drongo/wallet/StandardAccount.java>
- Historical Sparrow Whirlpool mix-status code:
  <https://code.sparrowwallet.com/sparrowwallet/sparrow/src/commit/78f0721168f8035f418f0a01fb89ea8e942038c0/src/main/java/com/sparrowwallet/sparrow/control/MixStatusCell.java>
  and
  <https://code.sparrowwallet.com/sparrowwallet/sparrow/blame/commit/176e440195f975253cdfeab08636b1a897bf78a5/src/main/java/com/sparrowwallet/sparrow/wallet/UtxoEntry.java>
- BIP329 labels JSONL: <https://bips.xyz/329>
- BIP352 Silent Payments: <https://github.com/bitcoin/bips/blob/master/bip-0352.mediawiki>
- BIP392 `sp()` descriptors: <https://github.com/bitcoin/bips/blob/master/bip-0392.mediawiki>

## Silent Payments watch-only sources

Kassiber can track BIP352 receives as a local-first, watch-only wallet source.
This is accounting infrastructure only: it does not spend, sign, broadcast,
construct PSBTs, manage sender contacts, or choose a hosted Silent Payments
server for you.

Create a wallet with BIP392 watch-only material and an explicitly selected
backend:

```bash
kassiber backends create sp-local \
  --kind custom \
  --url local://silent-payments \
  --chain bitcoin \
  --network main \
  --silent-payments \
  --silent-payment-scan-file /path/to/sp-scan.json

kassiber wallets create \
  --label "SP receive" \
  --kind silent-payment \
  --backend sp-local \
  --sp-descriptor-stdin \
  --sp-scan-start-height 850000
```

`--sp-descriptor` accepts watch-only `sp(spscan...)` material, or the BIP392
two-key watch-only shape where the scan key is private and the spend key is
public. Spend-private forms such as `spspend` are rejected. Prefer
`--sp-descriptor-stdin`, `--sp-descriptor-fd`, or `--sp-descriptor-file` over
argv so scan material does not land in shell history.

Every Silent Payments wallet must declare a scan birthday: either
`--sp-scan-start-height`, `--sp-scan-start-date`, or explicit full-history mode
with `--sp-full-history --sp-acknowledge-full-history-warning`. Kassiber does
not silently scan from genesis and does not claim completeness before the
scanner reports the requested range complete.

Backends must be deliberately marked Silent-Payments-capable with
`--silent-payments` and must provide either a local scanner JSON file
(`--silent-payment-scan-file`) or a server-assisted scan endpoint
(`--silent-payment-scan-path` plus wallet `--sp-scan-mode server-assisted
--sp-acknowledge-server-warning`). The desktop backend settings dialog exposes
the same capability bit and replacement scan file/path fields; already-saved
scanner paths remain hidden in normal safe reads. Ordinary Esplora/Electrum
scripthash sync is not enough to discover BIP352 outputs; unsupported backends
fail with `silent_payment_backend_unsupported` rather than returning a clean
zero balance.

Server-assisted scans are a trust/completeness tradeoff, not just a transport
choice. The selected backend may learn enough to correlate the wallet, and if
it omits Silent Payments scan candidates, Kassiber cannot independently prove
that a reported-complete range found every payment. Prefer a local scanner or a
self-hosted SP indexer for accounting-critical books. Server-assisted scanner
endpoints must be HTTP(S); Electrum `ssl://` / `tcp://` backends can only be
used with local scanner-file mode.

The local scanner file is outside Kassiber's SQLite/SQLCipher boundary. Treat
it like wallet metadata: keep it in a private, local directory and do not place
it in shared, cloud-synced, or world-readable locations. On POSIX systems,
Kassiber refuses to read scanner JSON files that are not regular files owned by
the current OS user, or that grant any group/other permissions; use `chmod 600`
for scanner output files before syncing.

The local scanner JSON shape is intentionally simple and scanner-agnostic:

```json
{
  "descriptor_fingerprint": "sha256-of-compact-sp-descriptor",
  "complete": true,
  "range": {"from_height": 850000, "to_height": 851000},
  "transactions": [
    {
      "txid": "64-hex...",
      "block_height": 850100,
      "block_time": "2026-06-01T12:00:00Z",
      "outputs": [
        {
          "vout": 0,
          "amount_sats": 50000,
          "script_pubkey": "5120...",
          "silent_payment": true
        }
      ]
    }
  ],
  "utxos": [
    {
      "txid": "64-hex...",
      "vout": 0,
      "amount_sats": 50000,
      "script_pubkey": "5120...",
      "spent_by": null
    }
  ]
}
```

Every scanner payload must be bound to the wallet with `descriptor_fingerprint`
(the hex SHA-256 of the whitespace-compacted `sp(...)` descriptor) or a
matching `wallet_id` / `kassiber_wallet_id`. A `wallet_label` may also be
included and is rejected when it mismatches, but labels are profile-local and
are not accepted as the only binding. The binding fields may be top-level or
inside a top-level `wallet` object. A mismatched or unbound payload is rejected
so one wallet cannot accidentally ingest another scanner result.

Detected outputs must include concrete Taproot scriptPubKeys and must be
explicitly marked with `silent_payment: true`, `owned: true`, or
`matched: true`; unmarked transaction outputs are ignored. Top-level `utxos`
must correspond to those marked transaction outputs or the scan is rejected.
Spend transactions that consume owned inputs must include `fee_sats` (or
`fee_sat`) and should mark owned inputs in `inputs` / `vin` with the same
ownership flags so the inventory can mark the previous outpoint spent.

Receives import as ordinary BTC inbound transactions and active UTXOs; later
spends mark the same UTXO spent and import an outbound BTC transaction.
Re-running sync is idempotent. The reported `range.from_height` /
`range.start_height` or `range.from_date` / `range.start_date` must cover the
wallet's configured scan birthday. If a scanner reports `complete=false`, a
too-narrow range, or another degraded state, Kassiber records partial success,
blocks report readiness for that source, and does not apply a full UTXO
snapshot update until the range completes.

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

## Generic ledger import

The generic ledger is a fill-in template for entering transactions by hand —
useful when there is no provider export, or for one-off corrections. It is the
column-mapped, Bitcoin-scoped front end over the generic import shape above, so
you do not have to know Kassiber's internal field names.

Write a blank template, then import the filled file:

```
kassiber wallets ledger-template --file ledger.xlsx   # or ledger.csv
kassiber wallets import-ledger --workspace W --profile P --wallet WID --file ledger.xlsx --dry-run  # preview
kassiber wallets import-ledger --workspace W --profile P --wallet WID --file ledger.xlsx            # import
```

`ledger-template` does not need a database. The `.xlsx` template ships a
`Transactions` sheet (with a `Type` dropdown) plus a `Legend` sheet; the `.csv`
template is the header row with example rows. The importer reads `.xlsx` (via
the `openpyxl` dependency) and CSV/TSV (delimiter sniffed). Legacy binary `.xls`
is not read — save as `.xlsx` or CSV first.

`--dry-run` previews what would import (also a no-DB read): it returns
`{rows_read, mapped, errors, problems[], preview[], confident, detected[]}` and
persists nothing, collecting a row-numbered problem for every rejected row at
once (the real import stops at the first bad row). In the desktop UI the same
flow is **Add connection → Files → Generic ledger**: after you choose the filled
file, a preview (backed by the `ui.wallets.ledger_preview` daemon kind) shows the
detected rows + any problems so you can confirm before importing.

### Bring your own file (auto-detected columns)

You do not have to use the template. A file that isn't in the template shape (no
`Type` + Received/Sent columns) is **auto-detected**: `infer_ledger_columns`
matches common header aliases — date; a `Type`/`Side` column, or a
direction/sign, or separate sent/received columns; fee; fiat currency/price/value;
note; transaction id — and remaps each row onto the ledger shape, so it imports
through the *same* normalizer and its `Type`→tax-kind taxonomy + exact pricing.
Rows without an explicit type become `Buy`/`Sell` when a cash counterleg is
present, or `Deposit`/`Withdrawal` by direction when only a valuation price/value
is present. When the columns can't
be recognized the import returns a `ledger_unrecognized` error (the desktop
preview shows it and points you back at the template); the dry-run/preview
returns `confident: false` with the detected columns instead of raising. Template
files are unaffected — they still take the native path.

### Columns

| Column | Meaning |
| --- | --- |
| `Type` | What happened (see the table below). Required. |
| `Date` | `YYYY-MM-DD`, a full timestamp, or `DD.MM.YYYY`. Required. |
| `Received Amount` / `Received Asset` | What came in. For a Buy: the Bitcoin you bought. |
| `Sent Amount` / `Sent Asset` | What went out. For a Buy: the fiat you paid. |
| `Fee Amount` / `Fee Asset` | Optional. A blank `Fee Asset` means the fee is in Bitcoin (on-chain/network fee). A fiat fee must be in the same currency as the row's fiat amount. |
| `Fiat Value` | Fair-market value in the book currency. Use it for Income/Mining/Spend/Gift rows that have no cash leg. |
| `Counterparty` | Optional. Exchange, merchant, or person. |
| `Note` | Optional free text, stored as the transaction description. |
| `Tx-ID` | Optional but recommended — the dedup key. Without it, rows dedup by their economic fingerprint (date/direction/asset/amount/fee), so two genuinely identical rows need a `Tx-ID` to be kept apart. |

Each row carries exactly **one Bitcoin leg** (`BTC`, `LBTC`, or `SATS`); the
other side, when present, is the fiat/cash leg that becomes exact
`exchange_execution` pricing. Crypto-to-crypto rows (Bitcoin on both sides) are
rejected. Amounts are in BTC (e.g. `0.05000000`) unless the asset is `SATS`,
in which case whole satoshis are converted. Numbers may use either a dot or a
comma decimal separator (`0,05` and `3.000,00` are read the same as `0.05` and
`3000.00`), so a sheet exported from a German/Austrian-locale spreadsheet
imports correctly. Fiat columns must be in the book's currency — a EUR book
rejects a row priced in JPY; import into a matching book or drop the fiat value
and let Kassiber price the row.

### Types

The `Type` maps to a direction plus a tax `kind`. Income kinds become RP2
earn-like receipts. The **outbound** disposals `Gift sent`/`Donation`/`Lost`/
`Stolen` are deliberately routed to the non-sale-disposal quarantine for
explicit review instead of being booked as ordinary market sales. `Gift
received` is the inbound counterpart and is booked as a plain acquisition at the
fair-market value you enter (`Fiat Value`), the same as a `Buy` without a cost.

| Group | Types | Direction |
| --- | --- | --- |
| Acquire | Buy, Deposit, Gift received | inbound |
| Dispose | Sell, Withdrawal, Spend | outbound |
| Earn | Income, Mining, Staking, Interest, Airdrop, Fork | inbound |
| Outflow (review) | Gift sent, Donation, Lost, Stolen | outbound |

To record moving Bitcoin between two of your own wallets, import a `Withdrawal`
into the source wallet and a `Deposit` into the destination wallet with the same
`Tx-ID`; transfer matching pairs them into a non-taxable move.

A row with an unrecognized `Type`, no Bitcoin leg, a missing `Date`, or a
direction that contradicts its `Type` fails the whole import with a
row-numbered, actionable message — fix the file and re-import (dedup makes
re-imports safe).

## Local-AI Photo/PDF OCR Drafts

For providers that only send a PDF statement, receipt image, or paper note,
Kassiber can ask a local Ollama vision/OCR model to draft generic transaction
rows:

```bash
kassiber wallets preview-document --file receipt.png --model glm-ocr
kassiber wallets import-document --wallet WID --file receipt.png --model glm-ocr
```

`preview-document` is read-only. It requires a local loopback AI provider
(`http://localhost:11434/v1` by default) and an installed vision/OCR model.
Recommended Ollama models are `glm-ocr`, `qwen3-vl:8b`,
`qwen3-vl:4b`, `llama3.2-vision:11b`, and `minicpm-v:8b`.
The desktop picker lists only configured local providers; remote providers are
never offered for document extraction.

The preview returns draft rows with `ready` / `quarantined` status,
per-row and per-cell confidence, evidence text, source regions when the model
provides them, and a normalized `import_record`. `import-document` imports
only ready rows unless `--include-quarantined` is passed, then copies the
source image/PDF into the managed attachments tree for every inserted or
enriched transaction. The preview carries a SHA-256 digest of the local source,
and import rejects the draft if that file changed between review and commit.

Off-device AI providers are hard-disabled for this path even if they are
configured for chat. URLs are also rejected: open Google Drive/Docs links in
the logged-in browser, download the PDF/image, and import the local file.

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

`wallets import-samourai` is a local watch-only importer for historical
Samourai Wallet activity. Kassiber accepts explicit public descriptors and
account xpub-family keys for the relevant Samourai/Whirlpool accounts, then
persists the redacted wallet configuration needed for ordinary descriptor sync.
It does not accept encrypted backups, recovery words, BIP39 passphrases, private
keys, or other secret-bearing wallet material. It does not connect to a
Whirlpool coordinator, does not run an active mixing client, does not create or
broadcast transactions, and does not expose descriptors, xpubs, PayNym secrets,
backend URLs/tokens, or raw source-set files through daemon results, AI tools,
diagnostics, docs, or tests.

Source findings that shape the import:

- Drongo's `StandardAccount` defines Whirlpool accounts as native segwit
  account roots: Badbank `2147483644'`, Premix `2147483645'`, and Postmix
  `2147483646'`; Postmix uses a minimum lookahead twice the normal default.
- Samourai's Whirlpool docs define Deposit, Premix, and Postmix as segregated
  address spaces in the same wallet. Deposit contains
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
That command returns structured fields (`wallet_material`, `descriptor`,
`change_descriptor`, and any related local-only material). For
checksum-sensitive imports into another wallet, copy only the raw descriptor
material: the descriptor string itself, or the receive descriptor followed by
the change descriptor on the next line when both branches are stored. Do not
paste the `field:` label, table output, or JSON envelope. Kassiber preserves the
stored descriptor text for reveal. Descriptor checksums are ignored only while
parsing/deriving addresses internally.
Use `kassiber wallets reveal-descriptor --material-only --wallet <name>` for a
pasteable CLI payload with no labels or envelope.
The desktop Add Connection flow asks for the four primary public account inputs
directly: Deposit, Badbank / Toxic Change, Premix, and Postmix descriptors or
account xpub-family keys. Internally those fields are converted into the same
source-set structure accepted by the CLI and daemon.
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
  --label "Samourai Watch-Only" \
  --source-set-file /path/to/samourai-public-sources.json \
  --backend mempool \
  --gap-limit 80
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

The desktop Add Connection flow asks for the real Wasabi RPC outputs directly:
paste `gethistory` JSON, optionally paste `listcoins` / `listunspentcoins` and
`getwalletinfo`, and use the advanced additional-sections box for `listkeys`,
`listpaymentsincoinjoin`, or `wallet_json`. The UI converts those pasted
responses into the same sanitized bundle shape internally. A prebuilt local
bundle file is still accepted for advanced/import-script workflows.
Wasabi RPC is disabled by default, listens locally on `127.0.0.1:37128` when
enabled, and may be anonymous or protected with `JsonRpcUser` /
`JsonRpcPassword` Basic Auth in Wasabi's `Config.json`. If Basic Auth is
enabled, pass `-u user:password` to the local `curl` command before pasting the
result into Kassiber.

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

## Bull Bitcoin Wallet CSV

Bull's mobile wallet can export a unified wallet-history CSV containing Bitcoin
on-chain, Liquid, Lightning, payjoin, and swap rows. Kassiber supports this as
`source_format="bullbitcoin_wallet_csv"`. This is wallet activity, not exchange
order evidence, and it does not contain fiat execution prices.

Use this importer when the Bull wallet export is the source of wallet history.
Create one Kassiber wallet per Bull network from the same CSV so BTC, Liquid,
and Lightning rows stay separate. This lets the swap matcher see BTC/LBTC swap
legs as different wallets instead of two rows inside one mixed wallet:

```bash
python3 -m kassiber wallets create \
  --label "Bull Bitcoin Wallet - Bitcoin" \
  --kind bullbitcoin \
  --source-file /path/to/bull_transactions.csv \
  --source-format bullbitcoin_wallet_csv \
  --config '{"bullbitcoin_wallet_network":"bitcoin"}'

python3 -m kassiber wallets create \
  --label "Bull Bitcoin Wallet - Liquid" \
  --kind bullbitcoin \
  --source-file /path/to/bull_transactions.csv \
  --source-format bullbitcoin_wallet_csv \
  --config '{"bullbitcoin_wallet_network":"liquid"}'

python3 -m kassiber wallets sync --wallet "Bull Bitcoin Wallet - Bitcoin"
python3 -m kassiber wallets sync --wallet "Bull Bitcoin Wallet - Liquid"
```

Behavior:

- `bullbitcoin_wallet_network=bitcoin` imports only Bitcoin on-chain/payjoin and
  Bitcoin-side chain-swap rows as `BTC`
- `bullbitcoin_wallet_network=liquid` imports only Liquid and Liquid-side
  chain-swap rows as `LBTC`
- `bullbitcoin_wallet_network=lightning` imports Lightning rows as `BTC`
- Lightning rows derive Kassiber's `payment_hash` from a valid exported
  preimage, or fall back to a 64-hex `txid`, for exact swap-pair matching
- chain-swap metadata such as `swap_id`, `send_network`, `receive_network`,
  `send_txid`, and `receive_txid` is preserved in redacted raw metadata and
  feeds the exact `provider_swap_id` matcher for cooperative Taproot/key-path
  flows where chain data alone is not identifying
- `preimage` is not stored in raw metadata; the importer records that it was
  redacted
- `direction=self` rows and `status=failed` / `status=expired` /
  `status=refunded` rows are skipped because they are not standalone taxable
  wallet movements. Bull's CSV currently collapses a refunded chain swap to a
  single canonical swap row and does not export the refund txid as its own leg,
  so Bull CSV alone cannot prove or book the refund round trip. Use descriptor
  chain sync (for script-path HTLC refund evidence) or provider/SDK metadata
  that carries both the lockup and refund legs before pairing it as
  `swap-refund`.

If BTC and Liquid descriptors are already the book's source of wallet history,
do not also import the same Bull wallet CSV rows into separate active wallets,
or the on-chain/Liquid transactions will be duplicated. Attach the unified Bull
export to those existing wallets instead:

```bash
python3 -m kassiber wallets attach-bullbitcoin-wallet \
  --wallet "Bitcoin descriptor" \
  --file /path/to/bull_transactions.csv \
  --network bitcoin

python3 -m kassiber wallets attach-bullbitcoin-wallet \
  --wallet "Liquid descriptor" \
  --file /path/to/bull_transactions.csv \
  --network liquid

python3 -m kassiber wallets sync --wallet "Bitcoin descriptor"
python3 -m kassiber wallets sync --wallet "Liquid descriptor"
```

The next wallet refresh keeps the descriptor/file source authoritative and uses
matching Bull rows only to backfill safe metadata such as swap kind, redacted
raw Bull route data, and Lightning payment hashes. Unmatched Bull rows are left
uninserted in this mode.

If the CSV was imported first and descriptors are added later, map the Bull CSV
onto the descriptor wallets with `attach-bullbitcoin-wallet`, refresh the
descriptor wallets, then retire or delete the earlier Bull CSV source wallets
before relying on portfolio totals. Kassiber does not automatically migrate
transactions from the CSV source wallets into descriptor wallets.

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

## Ledger Live

Kassiber supports Ledger Live operation-history CSV as wallet movement only.
Ledger's own export warns that countervalues are informational, so Kassiber
imports BTC/LBTC `IN` and `OUT` rows without exact fiat pricing. Account xpub
columns are redacted from stored `raw_json`.

```bash
python3 -m kassiber wallets import-ledger-live \
  --workspace W --profile P --wallet "Ledger Live" \
  --file /path/to/ledger-live.csv
```

You can also store `source_format="ledgerlive_csv"` on a wallet and use
`wallets sync`. Non-BTC/LBTC assets are skipped. Unsupported Ledger operation
types fail with `AppError` instead of being guessed.

## DALI/RP2-Inspired Exchange Imports

Kassiber ports the useful BTC-focused DALI/RP2 row taxonomy natively instead of
depending on DALI at runtime. Exchange/order evidence stays separate from later
self-custody wallet sync evidence.

### Kraken API

Create a backend with `kind=kraken`; store the API key in `token` and the
base64 API secret in `auth_header` (or `password`). Raw credentials remain in
the backend secret boundary and are redacted from output.

```bash
python3 -m kassiber backends create kraken-main \
  --kind kraken --url https://api.kraken.com \
  --token-stdin --auth-header-stdin

python3 -m kassiber wallets sync-kraken \
  --workspace W --profile P --backend kraken-main
```

The importer fetches Kraken private `TradesHistory` and `Ledgers`, imports
BTC/LBTC deposits and withdrawals as wallet movement, and imports fiat-quoted
BTC/LBTC trades as exact `exchange_execution` pricing (`pricing_method =
"kraken_api"`). BTC trades without matching trade history or without a fiat
quote fail safe.

### Coinbase API

Create a backend with `kind=coinbase`; store the API key in `token` and the
API secret in `auth_header` (or `password`).

```bash
python3 -m kassiber backends create coinbase-main \
  --kind coinbase --url https://api.coinbase.com \
  --token-stdin --auth-header-stdin

python3 -m kassiber wallets sync-coinbase \
  --workspace W --profile P --backend coinbase-main
```

BTC account `buy`, `sell`, `trade`, and `advanced_trade_fill` rows import as
exact execution evidence when Coinbase supplies a usable fiat native amount.
BTC sends/exchange transfers import as wallet movement without exact execution
pricing. Unsupported BTC row types raise validation errors rather than guessing.

### Binance API and Supplemental CSV

Create a backend with `kind=binance`; store the API key in `token` and the API
secret in `auth_header` (or `password`).

```bash
python3 -m kassiber backends create binance-main \
  --kind binance --url https://api.binance.com \
  --token-stdin --auth-header-stdin

python3 -m kassiber wallets sync-binance \
  --workspace W --profile P --backend binance-main
```

The native API importer covers BTC fiat-payment buys, BTC deposits,
withdrawals, and BTC income/dividend rows. Binance spot pair crawling,
altcoin/BNB dust conversions, staking-lock principal bookkeeping, and mining
subaccounts that require an extra username are intentionally deferred because
they are either altcoin-heavy or need additional provider-specific controls.

Supplemental Binance CSV import currently supports:

- autoinvest BTC rows funded by fiat (`binance_supplemental_csv`) as exact
  execution evidence
- BTC dividend/mining-style rows as income without exact fiat pricing

```bash
python3 -m kassiber wallets import-binance-supplemental \
  --workspace W --profile P \
  --file /path/to/binance-supplemental.csv
```

Crypto-funded Binance autoinvest/cross-asset rows fail validation and should be
entered through the generic ledger with explicit review. DALI's Pionex, Nexo,
BlockFi, and Trezor-family plugins are deferred for Kassiber: they are
obsolete, altcoin-heavy, or add less value than descriptor/wallet sync plus the
generic ledger for BTC-side edge cases.

## BIP329

Kassiber stores imported BIP329 records once per active profile, deduplicated by
record type and reference. Re-importing the same reference updates the stored
label metadata instead of creating a second wallet-scoped copy. The importer
accepts `tx`, `addr`, `pubkey`, `input`, `output`, `xpub`, and `spscan`
records, preserves unknown JSONL fields, and adds a small
`kassiber.wallet_match` extension so exact/ambiguous/unmatched ownership
decisions can round-trip.

Preview before applying:

```bash
python3 -m kassiber metadata bip329 preview --file /path/to/labels.jsonl
```

The preview reports exact matches, ambiguous matches, unmatched records,
duplicates, conflicts with existing BIP329 rows, and transaction-tag effects.
Import remains profile-wide and stores every valid BIP329 record, but only exact
transaction matches are projected into Kassiber tags by default. Ambiguous
transaction labels are preserved for export/review and skipped unless the CLI
caller explicitly opts in:

```bash
python3 -m kassiber metadata bip329 import --file /path/to/labels.jsonl
python3 -m kassiber metadata bip329 import --file /path/to/labels.jsonl --apply-ambiguous
python3 -m kassiber metadata bip329 list
```

Exports can replay stored BIP329 rows, synthesize wallet-readable transaction
labels from Kassiber metadata, or combine both. Profile-wide export includes all
stored rows. Wallet-scoped export includes only rows Kassiber can tie to the
chosen wallet with deterministic ownership; ambiguous and unmatched stored rows
are excluded rather than guessed.

```bash
python3 -m kassiber metadata bip329 export --mode stored --file /path/to/export.jsonl
python3 -m kassiber metadata bip329 export --mode synthesized --wallet coldcard --file /path/to/coldcard-labels.jsonl
python3 -m kassiber metadata bip329 export --mode all --file /path/to/book-labels.jsonl
```

Synthesized labels are human-readable wallet context first. Kassiber may include
standard optional BIP329 fields such as `time`, `value`, `fee`, and `rate` when
they come from local transaction metadata, plus a namespaced `kassiber` object
for Kassiber-aware round trips. Third-party wallets can ignore unknown fields
and still ingest the JSONL as normal BIP329 labels. BIP329 files are sensitive:
they can expose txids, addresses, xpub/silent-payment scan material, labels, and
intent.

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
python3 -m kassiber attachments rename <ATTACHMENT_ID> --label "Accountant approval"
```

File attachments are copied into Kassiber's managed attachment store. URL
attachments are stored literally and are not fetched. Kassiber shows a display
label derived from the URL, which you can edit without changing the URL target.
