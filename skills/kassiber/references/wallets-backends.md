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

For secret-bearing setup, prefer giving the user a paste-ready command template
to run in a separate local terminal instead of asking them to paste descriptors,
tokens, or other credentials into chat.

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

Paste-ready local templates:

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
```

`kassiber wallets get` returns an allowlisted safe config view. Use
`descriptor`, `change_descriptor`, and `descriptor_state` to confirm wallet
state instead of expecting the raw descriptor back or arbitrary config keys
to be echoed.

`wallets sync` takes `--wallet <label-or-id>` or `--all`; the wallet is not a
positional argument.

## Imports

Import into an existing wallet when the file represents the same real wallet.

BTCPay:

```bash
kassiber wallets import-btcpay --wallet btcpay --file /path/to/export.csv --input-format csv
kassiber backends create btcpay-prod --kind btcpay --url https://btcpay.example.com --token "$BTCPAY_TOKEN"
kassiber wallets create --label btcpay-shop --kind custom --backend btcpay-prod --store-id <store-id>
kassiber wallets sync --wallet btcpay-shop
kassiber wallets sync-btcpay --wallet btcpay-shop --backend btcpay-prod --store-id <store-id>
```

`wallets sync-btcpay` keeps the old explicit shape, but it now stores the same
BTCPay backend/store config on the wallet so later `wallets sync` and
`wallets sync --all` can reuse it.

Do not ask users to paste raw BTCPay API tokens into chat. Prefer a local shell
variable such as `BTCPAY_TOKEN`, a local `backends.env` entry, or a command
they run locally with the secret substituted on their machine.

Paste-ready local template:

```bash
kassiber backends create <btcpay-backend-name> \
  --kind btcpay \
  --url <btcpay-base-url> \
  --token "$BTCPAY_TOKEN"
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

Generic files:

```bash
kassiber wallets import-json --wallet wallet-name --file /path/to/data.json
kassiber wallets import-csv --wallet wallet-name --file /path/to/data.csv
```

Do not create a second wallet for a BTCPay or Phoenix export when it belongs to a wallet already tracked in Kassiber.
Do not create one Kassiber wallet per BTCPay store if multiple stores share the same underlying wallet balance.

## Austrian profiles

Kassiber does not currently expose Austrian-specific wallet provenance controls.

If the user asks about Austrian tax handling, explain that `tax_country=at`
is supported through the Kassiber-maintained RP2 fork at
`bitcoinaustria/rp2`.

Current limits to mention:

- Austrian cross-asset `--policy carrying-value` pairing is supported.
- Austrian E 1kv export is available through `reports austrian-e1kv` and
  `reports export-austrian-e1kv-pdf`, but domestic-provider withheld KESt
  metadata is not modeled yet.
- If the installed `rp2` environment lacks `rp2.plugin.country.at`, stop and
  fix the environment instead of guessing.

Do not say BTC ↔ LBTC swaps are already handled just because the profile is
Austrian. The operator still needs an explicit `kassiber transfers pair` for
cross-asset peg-ins / peg-outs before the carry-basis path can show up in
journal state.
