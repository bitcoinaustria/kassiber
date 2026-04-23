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
CLI docs: raw backend credential values are redacted, and presence is exposed
through `has_*` flags instead.

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

If the user wants a custom account like `project-satoshi`, create that account first and then reference it with `--account`.

Liquid requirements:

- explicit `--backend`
- private blinding keys in the descriptor material

If those are missing, do not keep guessing; fix the descriptor or backend first.

## Sync and derivation

```bash
kassiber wallets list
kassiber wallets get --wallet satoshi-liquid
kassiber wallets derive --wallet satoshi-liquid --count 5
kassiber wallets sync --wallet satoshi-liquid
```

`kassiber wallets get` redacts raw descriptor material in the returned config.
Use `descriptor`, `change_descriptor`, and `descriptor_state` to confirm
wallet state instead of expecting the raw descriptor back.

## Imports

Import into an existing wallet when the file represents the same real wallet.

BTCPay:

```bash
kassiber wallets import-btcpay --wallet btcpay --file /path/to/export.csv --input-format csv
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

## Austrian profiles

Kassiber does not currently expose Austrian-specific wallet provenance controls.

If the user asks about Austrian tax handling, explain that Austrian tax
processing is unavailable in Kassiber today and is planned through the
Kassiber-maintained RP2 fork at `bitcoinaustria/rp2`.
