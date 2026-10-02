# Wallets and Backends

Use this reference for wallet onboarding, descriptor setup, backend selection, wallet imports, and wallet sync.

## Backends

Backends are Kassiber's sync endpoints. List and inspect them first:

```bash
kassiber backends list
kassiber backends kinds
kassiber backends get liquid
```

These inspection commands follow the same safe-to-record contract as the main
CLI docs: backend inspection returns an allowlisted safe view, raw backend
credential values and unknown config keys are suppressed, and presence is
exposed through `has_*` flags instead.

Common backend operations:

```bash
kassiber backends create my-esplora --kind esplora --url https://example.invalid/api
kassiber backends update my-esplora --url https://new.example.invalid/api
kassiber backends update core --clear username --clear password --clear cookiefile
kassiber backends set-default my-esplora
```

Behavior to remember:

- read-only commands keep bootstrap-backed config in memory only; `kassiber init` and backend mutation commands that need canonical bootstrap rows are the explicit bootstrap-import flows
- deleting a bootstrap-backed backend suppresses the built-in/default bootstrap copy, but a backend present in the current `backends.env` file is treated as an explicit restore signal
- process-level `KASSIBER_BACKEND_*` overrides still win for the current process over the stored SQLite row

Built-in defaults often include:

- `mempool` for Bitcoin Esplora
- `fulcrum` for Bitcoin Electrum
- `liquid` for Liquid Electrum

If a sync failure suggests trying a different backend or a larger gap limit,
diagnose first and confirm with the user before persisting that change with
`wallets update` or `backends set-default`. Those are durable config mutations,
not throwaway retries.

## Wallet kinds

Discover available kinds with:

```bash
kassiber wallets kinds
```

Common kinds for the workflows in this skill:

- `descriptor`
- `address`
- `phoenix`
- `custom`

`kassiber wallets kinds` currently exposes additional kinds too, including `xpub`, `coreln`, `lnd`, `nwc`, and `river`. Trust the CLI output if it differs from this focused shortlist.

## Connection handoff

When the user wants help connecting a wallet or backend and the exact source
type is still unclear, ask for the connection type first. Good examples are
descriptor wallet, BTCPay, Phoenix import, Bitcoin RPC, or Electrum/Esplora
backend.

Assume a mainnet connection unless the user explicitly says testnet, signet,
regtest, or another non-mainnet environment. For Liquid, that means the normal
mainnet pair `--chain liquid --network liquidv1`.

For desktop setup, prefer the Connections setup modal so the user enters wallet
exports and local file paths into the local app instead of copying shell
commands. Backend-backed connections should select an already configured
backend; if none exists, route to Settings/backends. For CLI-only handoff, use
placeholders or `--*-stdin` / `--*-fd FD` forms for secrets; do not ask users
to paste descriptors, tokens, or credentials into chat.

The Connections modal should ask for one wallet export / descriptor field, not
separate receive and change descriptors. The daemon normalizes common formats
such as Bitcoin Core descriptor JSON, two-line descriptor text, key/value
descriptor exports, and ypub/zpub/upub/vpub single-sig keys.

A receive-only descriptor (just the `/0/*` chain) is sufficient: Kassiber
automatically derives the sibling `/1/*` change chain, so change UTXOs appear
in balances and the UTXO list without a separate change descriptor. This covers
single-sig, multisig, and Liquid. Supply `--change-descriptor` /
`--change-descriptor-file` (or a `<0;1>` multipath descriptor) only for a
non-standard change chain.

## Descriptor wallets

Bitcoin example:

```bash
kassiber wallets create \
  --label vault \
  --kind descriptor \
  --account treasury \
  --backend mempool \
  --descriptor-file /path/to/receive.desc \
  --change-descriptor-file /path/to/change.desc
```

Liquid example:

```bash
kassiber wallets create \
  --label satoshi-liquid \
  --kind descriptor \
  --account treasury \
  --backend liquid \
  --chain liquid \
  --network liquidv1 \
  --descriptor-file /path/to/receive.desc \
  --change-descriptor-file /path/to/change.desc
```

If the user wants a custom wallet/reporting bucket like `project-satoshi`, create that bucket with `accounts create` first and then reference it with `--account`.

Liquid requirements:

- explicit `--backend`
- private blinding keys in the descriptor material

If those are missing, do not keep guessing; fix the descriptor or backend first.

If the user already provided a secret-bearing Liquid descriptor such as
`ct(slip77(...),...)`, do not ask them to restate the private blinding key
separately and do not repeat the secret back in summaries.

If the Liquid wallet comes as a standard receive/change pair, map `/0/*` to the
main descriptor and `/1/*` to `--change-descriptor` or
`--change-descriptor-file`. Do not create two wallets just because both
branches are present.

CLI-only templates:

Bitcoin descriptor wallet:

```bash
kassiber wallets create \
  --label <wallet-label> \
  --kind descriptor \
  --account <bucket-code> \
  --backend mempool \
  --descriptor-file <receive-descriptor-file> \
  --change-descriptor-file <change-descriptor-file>
```

Liquid descriptor wallet:

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

The agent should hand these back as local fill-in templates rather than asking
the user to paste descriptor contents into chat.

## Sync and derivation

```bash
kassiber wallets list
kassiber wallets get --wallet satoshi-liquid
kassiber wallets derive --wallet satoshi-liquid --count 5
kassiber wallets sync --wallet satoshi-liquid
kassiber --machine wallets sync --all
```

`kassiber wallets get` returns an allowlisted safe config view. Use
`descriptor`, `change_descriptor`, and `descriptor_state` to confirm wallet
state instead of expecting the raw descriptor back or arbitrary config keys
to be echoed.

`wallets sync` takes either `--wallet <label-or-id>` or `--all`, never both;
the wallet is not a positional argument.
For "sync the project wallets" or "sync current wallets", use `--all` directly
unless the user names a single wallet.

## Ownership reconciliation (`wallets identify`)

Check whether a pile of addresses and/or transaction ids belong to any wallet
in the active profile — the workflow for telling apart historic payments from
transfers between your own wallets.

```bash
# Mixed inputs; --candidate auto-detects address vs txid, --file reads one per line
kassiber wallets identify --address bc1q... --txid <64-hex> --candidate <addr-or-txid>
kassiber wallets identify --file ./to-reconcile.txt
# Smart CSV import: harvests addresses/txids from any common shape
kassiber wallets identify --csv ./export.csv
# Spreadsheet-friendly annotated output
kassiber --format csv wallets identify --file ./to-reconcile.txt --output owned.csv
# Per-leg payment/transfer classification for txids not in local history (hits a backend)
kassiber wallets identify --txid <64-hex> --verify-on-chain --verify-backend mempool
```

- Matching is on canonical scriptPubKey (with address-string fallback for
  Liquid confidential addresses) and covers receive **and** change. Each owned
  result names the wallet, branch and derivation index; externals are flagged.
- Descriptor wallets are derived offline up to `--scan-to-index` (default 500),
  floored at the highest synced index. Raise it for deep historic reconciliation
  when a flagged address might sit far past the synced range.
- Without `--verify-on-chain`, a txid not already synced/imported returns
  `unknown` (cache-only, no network). `--verify-on-chain` fetches it through an
  Esplora/Electrum backend and classifies it as a self-transfer, outbound
  payment, or inbound receipt. JSON (`--machine`) keeps the full per-leg detail;
  `--format csv` flattens to one row per input.
- `--csv <path>` is a smart importer: it sniffs the delimiter (comma/semicolon/
  tab/pipe), strips a BOM, recognizes common `address`/`txid` headers, and
  otherwise content-harvests any cell that is a 64-hex txid or a real
  (checksum-validated) address — ignoring amounts, dates, memos, and labels. It
  handles header-less and one-per-line files too.
- A BOLT11 Lightning invoice is decoded on the device and matched by its
  payment hash against the book's Lightning history: it comes back `owned`
  (paid, received, or paid between your own wallets, naming the wallet) or
  `unknown` when no synced record carries that payment hash. The invoice itself
  is never echoed back in full.
- A BIP352 silent-payment address (`sp1…`) is matched against the profile's
  silent-payment wallets by its scan and spend keys, including labels 0–50
  (0 is the change label). Both keys must match for `owned`; the wallet's scan
  key with an unchecked spend key, or material that cannot be decoded, stays
  `unknown` rather than `external`.
- Recognized formats the check does not handle yet come back `unsupported`
  with their type named instead of being called external addresses: BOLT12
  offers, LNURLs, Lightning addresses, outpoints, and extended public keys.
  Private keys are also `unsupported`, and their value is never echoed back.
- Scope with repeatable `--wallet` (default: all wallets). At least one of
  `--address` / `--txid` / `--candidate` / `--file` / `--csv` is required.
- The desktop **Reconcile** screen is the GUI peer: it runs the cache-only
  check inline and offers a "Verify on chain" button for any `unknown` txids
  (daemon kinds `ui.wallets.identify` and the mutating `ui.wallets.identify_onchain`,
  which streams progress and can be stopped between lookups).

## Imports

Import into an existing wallet when the file represents the same real wallet.

BTCPay (Greenfield API):

```bash
# 1. Permissions and a pre-filled BTCPay link; no network request.
kassiber btcpay key-url --server-url https://btcpay.example.com --preset read_only
# 2. Save the key the user created (never paste it into chat).
printf %s "$BTCPAY_TOKEN" | kassiber backends create btcpay-prod \
  --kind btcpay --url https://btcpay.example.com --token-stdin
# 3. Read-only inspection: key scope/risk, stores, payment methods, shared
#    store wallets, recognised tracked wallets, store freshness, and a
#    suggested plan.
kassiber btcpay inspect --backend btcpay-prod
# 4. Apply the plan, or override single payment methods.
kassiber btcpay setup --backend btcpay-prod --label Shop --recommended --dry-run
kassiber btcpay setup --backend btcpay-prod --label Shop \
  --route <store-id>:BTC-CHAIN=existing_wallet@<wallet-label> \
  --route <store-id>:BTC-LN=payment_ledger
kassiber wallets sync --all
kassiber btcpay provenance sync --all
```

Key presets: `read_only` (`btcpay.store.canviewstoresettings`) reads invoices,
payments, refunds/payouts, payment requests, and the address preview used to
recognise which wallet a store pays into; it cannot change the store.
`wallet_history` (`btcpay.store.canmodifystoresettings`) is the only way BTCPay
exposes its own on-chain wallet history, labels, and payout fees, and it can
also change the store's payout wallet. BTCPay's own records are an accurate
balance source; do not push users towards watch-only wallets. Explain the
permission trade-off and let the user choose: a wallet-history key, or mapping
the store to a wallet they already track.

Setup actions per store payment method: `wallet_source` (import BTCPay wallet
history), `existing_wallet` (the store pays into a wallet Kassiber tracks),
`payment_ledger` (Lightning, LNURL, and bitcoin plugin rails Kassiber cannot
watch: BTCPay's settled payments and completed payouts become the balance),
`provenance_only` (Lightning already booked by a connected node, stores sharing
an already imported wallet), `skip`. A payment ledger only sees what passes
through BTCPay; withdrawals made outside it need their own record, and it needs
a key that can read payouts. A configured wallet source or ledger stays until
its wallet is archived. One wallet
is imported once: stores that share a wallet, or the same store reached through
a second API key, must not become two wallet sources or two ledgers; the core
rejects such plans. Each API key is its own backend; one key can serve many
stores.

BTCPay does not push updates. Store data is only as current as the last sync:
check `sync_state` / the `stale_store_data` warning in `btcpay inspect` and
suggest a sync before period-end answers that depend on BTCPay data.

`wallets import-btcpay --file` remains the CSV path. `wallets create --store-id`
and `wallets sync-btcpay` keep the older single-store shape and store the
same config, so later `wallets sync` reuses it. Wallet syncs also refresh that
store's invoices and payouts and create local link suggestions; review them with
`btcpay provenance links --state suggested` and `btcpay provenance review`
(refunds and payouts review as `refund` or `expense` against the outbound
transaction, never as income; several payouts paid by one send review together
at their combined value). Sends that completed payouts explain book the miner
fee separately.

Do not ask users to paste raw BTCPay API tokens into chat. Prefer
`--token-stdin` (with a local `printf %s "$VAR" | kassiber ...` pipe) or
`--token-fd <FD>`. The argv form `--token <value>` still works for legacy
scripts but warns and leaks to shell history.

Phoenix:

```bash
kassiber wallets import-phoenix --wallet phoenix --file /path/to/export.csv
```

River:

```bash
kassiber wallets import-river --wallet river --file /path/to/river-account-activity.csv
kassiber wallets create \
  --label river \
  --kind river \
  --source-file /path/to/river-account-activity.csv \
  --source-format river_csv
kassiber wallets sync --wallet river
```

Prefer River Account Activity CSV when available because it includes both BTC
and cash legs. Kassiber skips fiat-only cash rows and preserves buy/sell cash
legs as exact `exchange_execution` pricing from provider `River`; BTC-only rows
with an exported Bitcoin price use that value as a River `fmv_provider` sample.

Bull Bitcoin / Coinfinity exchange evidence:

```bash
kassiber wallets import-bull --file /path/to/bull-orders.csv
kassiber wallets import-coinfinity --file /path/to/coinfinity-orders.csv
```

These imports default to `--mode relevant`: they enrich unique matching wallet
transactions anywhere in the current book and do not create standalone rows.
Use `--mode full` only when the shared provider export itself should be kept as
excluded evidence with matched / wallet-gap / ambiguous reconciliation tags.

CoinTracking / Blockpit migration history:

```bash
kassiber wallets import-cointracking --wallet cointracking --file /path/to/cointracking.csv
kassiber wallets import-blockpit --wallet blockpit --file /path/to/blockpit.csv
kassiber documents import-report --file /path/to/prior-tax-report.pdf --provider cointracking
```

Use a dedicated wallet of kind `cointracking` or `blockpit`. The provider CSV
and descriptor wallets may be added in either order: authoritative wallet sync
excludes exact transaction-id matches, holds id-less economic collisions for
review, and leaves provider-only history active. CoinTracking needs an English
Transactions CSV; Blockpit needs an unfiltered Transactions CSV. A prior report
is archived evidence only and does not create transactions.

Generic files:

```bash
kassiber wallets import-json --wallet wallet-name --file /path/to/data.json
kassiber wallets import-csv --wallet wallet-name --file /path/to/data.csv
```

Manual entry (no provider export, or one-off corrections): the generic ledger
is a fill-in Excel/CSV template whose `Type` column (Buy/Sell/Deposit/
Withdrawal/Spend/Income/Mining/Gift/…) maps onto real `(direction, kind)`
pairs. One Bitcoin leg per row; the fiat side becomes exact execution pricing.

```bash
kassiber wallets ledger-template --file ledger.xlsx      # blank template (.xlsx or .csv)
kassiber wallets import-ledger --wallet wallet-name --file ledger.xlsx
```

Amounts are in BTC (or whole sats when the asset is `SATS`); fiat columns must
match the book currency; gift/donation/lost/stolen rows are quarantined for
review. Full column + Type reference: [imports.md](../../../docs/reference/imports.md#generic-ledger-import).

### A provider with no predefined importer

Do not tell the user to retype or reshape their export. Any CSV/TSV/XLSX can go
through the generic ledger importer by mapping its columns:

```bash
kassiber wallets analyze-file --file export.csv   # headers + inferred plan + dry-run; no DB, no network
kassiber wallets import-ledger --wallet wallet-name --file export.csv \
  --column-map '{"date":"Trade Date","type":"Action","amount":"Qty","fiat_value":"Total"}'
```

`analyze-file` first. If it reports `confident: false`, or rejects rows for an
unrecognized `Type`, build the plan **from the headers it returned** and analyze
again with `--column-map` until `next_step.action` is `import`. Add `type_map`
inside the plan when the file labels rows in another language or house style
(`{"type_map": {"ACQ-MKT": "Buy"}}`); German values are already recognized.

When `row_kinds_from_amount_sign` is true, no Type or direction column was
recognized and every row would import as a plain transfer picked by the amount's
sign — a sale would book as a deposit with nothing rejected. Map the file's own
Type column, or confirm with the user that the export really is transfers only.

When `amount_units_unconfirmed` is true, nothing in the file says what the
amounts are denominated in and they are too large to be BTC — almost certainly
satoshis. Ask the user which rail it is; never assume. When `header_is_preamble`
is true the first row is an account-holder/date-range line rather than column
names, so no mapping can work: ask the user to delete the lines above the real
header and re-export.

In chat, the equivalent is the `ui.wallets.analyze_file` tool over the file the
user attached — same loop, same `column_map`, with one difference that matters:
**you propose the plan, the user runs the import.** There is no import tool in
chat, so once the analysis reaches `next_step.action = import`, give the user the
`wallets import-ledger --column-map '<plan>'` command and say the run is undoable
(`imports list` / `imports rollback --batch <id> --confirm`). Never say you
imported anything.

You are mapping column names and label vocabulary. Never transcribe, re-type, or
compute an amount, date, or total yourself: the importer reads every value from
the file, which is what makes the import auditable. Asset hints
(`amount_header_asset` and friends) are refused from chat — if a numeric column
does not say which rail it holds, ask the user rather than declaring one. A
`type_map` is the one mapping that decides a tax kind, so state the mapping you
propose ("Kauf → Buy") and let the user confirm it.

If the provider is not running on the user's machine you will not be shown cell
values at all (`cell_values_withheld`), and the headers are all you need. When
`headers_withheld` is non-zero the file has no real header row (a preamble line,
or a headerless export) — ask the user what the columns are instead of guessing
from `[withheld]`.

A dedicated first-class importer is still a source-repo change; this path is the
stop-gap that makes the export usable today.

Do not create a second wallet for a BTCPay or Phoenix export when it belongs to a wallet already tracked in Kassiber.
Do not create one Kassiber wallet per BTCPay store if multiple stores share the same underlying wallet balance.

## Austrian books

Kassiber does not currently expose Austrian-specific wallet provenance controls.

If the user asks about Austrian tax handling, explain that `tax_country=at`
is supported by Kassiber's built-in tax engine.

Current limits to mention:

- Austrian cross-asset `--policy carrying-value` pairing is supported.
- Austrian E 1kv export is available through `reports austrian-e1kv`,
  `reports austrian-tax-summary`, `reports export-austrian`,
  `reports export-austrian-e1kv-pdf`, `reports export-austrian-e1kv-xlsx`,
  and `reports export-austrian-e1kv-csv`, but domestic-provider withheld KESt
  metadata is not modeled yet.

Do not say BTC ↔ LBTC swaps are already handled just because the books are
Austrian. The operator still needs an explicit `kassiber transfers pair` for
cross-asset peg-ins / peg-outs before the engine's Austrian carry path can
show up in journal state.
