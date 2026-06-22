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
- Scope with repeatable `--wallet` (default: all wallets). At least one of
  `--address` / `--txid` / `--candidate` / `--file` / `--csv` is required.
- The desktop **Reconcile** screen is the GUI peer: it runs the cache-only
  check inline and offers a "Verify on chain" button for any `unknown` txids
  (daemon kinds `ui.wallets.identify` and the mutating `ui.wallets.identify_onchain`).

## Imports

Import into an existing wallet when the file represents the same real wallet.

BTCPay:

```bash
kassiber wallets import-btcpay --wallet btcpay --file /path/to/export.csv --input-format csv
printf %s "$BTCPAY_TOKEN" | kassiber backends create btcpay-prod \
  --kind btcpay --url https://btcpay.example.com --token-stdin
kassiber wallets create --label btcpay-shop --kind custom --backend btcpay-prod --store-id <store-id>
kassiber wallets sync --wallet btcpay-shop
kassiber wallets sync-btcpay --wallet btcpay-shop --backend btcpay-prod --store-id <store-id>
```

`wallets sync-btcpay` keeps the old explicit CLI shape, but it now stores the
same BTCPay backend/store config on the wallet so later `wallets sync` and
`wallets sync --all` can reuse it. Desktop setup should ask for store ID only
and let Kassiber use the default BTC on-chain payment method internally.

Do not ask users to paste raw BTCPay API tokens into chat. Prefer
`--token-stdin` (with a local `printf %s "$VAR" | kassiber ...` pipe) or
`--token-fd <FD>`. The argv form `--token <value>` still works for legacy
scripts but warns and leaks to shell history.

CLI-only template:

```bash
printf %s "$BTCPAY_TOKEN" | kassiber backends create <btcpay-backend-name> \
  --kind btcpay \
  --url <btcpay-base-url> \
  --token-stdin
kassiber wallets create \
  --label <wallet-label> \
  --kind custom \
  --backend <btcpay-backend-name> \
  --store-id <btcpay-store-id>
kassiber wallets sync --wallet <wallet-label>
kassiber wallets sync-btcpay \
  --wallet <wallet-label> \
  --backend <btcpay-backend-name> \
  --store-id <btcpay-store-id>
```

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

Generic files:

```bash
kassiber wallets import-json --wallet wallet-name --file /path/to/data.json
kassiber wallets import-csv --wallet wallet-name --file /path/to/data.csv
```

Do not create a second wallet for a BTCPay or Phoenix export when it belongs to a wallet already tracked in Kassiber.
Do not create one Kassiber wallet per BTCPay store if multiple stores share the same underlying wallet balance.

## Austrian books

Kassiber does not currently expose Austrian-specific wallet provenance controls.

If the user asks about Austrian tax handling, explain that `tax_country=at`
is supported through the Kassiber-maintained RP2 fork at
`bitcoinaustria/rp2`.

Current limits to mention:

- Austrian cross-asset `--policy carrying-value` pairing is supported.
- Austrian E 1kv export is available through `reports austrian-e1kv`,
  `reports austrian-tax-summary`, `reports export-austrian`,
  `reports export-austrian-e1kv-pdf`, `reports export-austrian-e1kv-xlsx`,
  and `reports export-austrian-e1kv-csv`, but domestic-provider withheld KESt
  metadata is not modeled yet.
- If the installed `rp2` environment lacks `rp2.plugin.country.at`, stop and
  fix the environment instead of guessing.

Do not say BTC ↔ LBTC swaps are already handled just because the books are
Austrian. The operator still needs an explicit `kassiber transfers pair` for
cross-asset peg-ins / peg-outs before rp2's native carry path can show up
in journal state.
