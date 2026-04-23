# Backends Reference

Kassiber syncs wallets through named backends. A backend is a pointer to an external indexer or node that Kassiber uses to discover transactions and balances.

Backends are stored canonically in SQLite.

- `~/.kassiber/config/backends.env` or your chosen `--env-file` is still
  accepted as a bootstrap / compatibility input
- the `backends` table in SQLite is the long-term source of truth

Built-in defaults and dotenv-defined backends are imported into SQLite during
explicit bootstrap-import flows such as `kassiber init` or backend mutation
commands that need a canonical SQLite row. Read-only commands keep that
bootstrap config in memory only. Environment-only overrides stay ephemeral
unless you explicitly create the backend through the CLI.

## Built-in defaults

Without any user configuration, Kassiber currently ships these built-in names:

- `mempool` -> `esplora` -> `https://mempool.bitcoin-austria.at/api`
- `fulcrum` -> `electrum` -> `ssl://index.bitcoin-austria.at:50002`
- `liquid` -> `electrum` -> `ssl://les.bullbitcoin.com:995`

`mempool` is the default for Bitcoin wallets.

## Useful commands

Inspect the merged backend view:

```bash
python3 -m kassiber backends list
python3 -m kassiber backends get mempool
```

Those inspection commands follow Kassiber's safe-to-record contract for
secret-bearing values: raw credentials are redacted, and presence is exposed
through `has_*` flags instead. If a backend URL contains embedded credentials
or query tokens, the displayed URL is sanitized before it is emitted.

Create and manage SQLite-backed backends:

```bash
python3 -m kassiber backends create myelectrum --kind electrum --url ssl://index.bitcoin-austria.at:50002
python3 -m kassiber backends update myelectrum --batch-size 50 --timeout 60
python3 -m kassiber backends update core --clear username --clear password --clear cookiefile
python3 -m kassiber backends create core --kind bitcoinrpc --url http://127.0.0.1:8332 --cookiefile ~/.bitcoin/.cookie --wallet-prefix kassiber
python3 -m kassiber backends set-default myelectrum
python3 -m kassiber backends clear-default
python3 -m kassiber backends delete myelectrum
```

Point a wallet at a named backend:

```bash
python3 -m kassiber wallets create \
  --label donations \
  --kind address \
  --backend mempool \
  --address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq
```

## Dotenv layout

The key pattern is:

- `KASSIBER_DEFAULT_BACKEND`
- `KASSIBER_BACKEND_<NAME>_<FIELD>`

Example:

```dotenv
KASSIBER_DEFAULT_BACKEND=mempool

KASSIBER_BACKEND_MEMPOOL_KIND=esplora
KASSIBER_BACKEND_MEMPOOL_CHAIN=bitcoin
KASSIBER_BACKEND_MEMPOOL_NETWORK=main
KASSIBER_BACKEND_MEMPOOL_URL=https://mempool.bitcoin-austria.at/api

KASSIBER_BACKEND_CORE_KIND=bitcoinrpc
KASSIBER_BACKEND_CORE_CHAIN=bitcoin
KASSIBER_BACKEND_CORE_NETWORK=main
KASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332
KASSIBER_BACKEND_CORE_COOKIEFILE=~/.bitcoin/.cookie
KASSIBER_BACKEND_CORE_WALLETPREFIX=kassiber
```

See [.env.example](../../.env.example) for a fuller template. Once imported,
use the `backends` CLI to inspect or edit the canonical SQLite rows.

Important runtime rules:

- read-only commands like `status`, `backends list`, and `backends get` do not import bootstrap-backed config into SQLite; `kassiber init` and backend mutation commands that need canonical bootstrap rows are the explicit bootstrap-import flows
- deleting a bootstrap-backed backend suppresses the built-in/default bootstrap copy, but a backend currently present in `backends.env` is treated as an explicit restore signal and will appear in the runtime view again
- process-level `KASSIBER_BACKEND_*` overrides still win for the current process even when a backend has already been imported into SQLite
- config-backed auth fields can be scrubbed with `backends update --clear ...`; clearing removes the stored key from SQLite instead of leaving the old value behind

## Supported backend kinds

Current sync backends:

- `esplora`
- `electrum`
- `bitcoinrpc`

Common fields:

- `KIND`
- `URL`
- `TIMEOUT`
- `CHAIN`
- `NETWORK`

Electrum-specific fields:

- `BATCH_SIZE`
- `INSECURE`

Bitcoin Core-specific fields:

- `USERNAME`
- `PASSWORD`
- `COOKIEFILE`
- `WALLETPREFIX`

Note: `bitcoinrpc` support is currently partial. Kassiber can use it for Bitcoin address-based wallets, but descriptor- and xpub-backed live sync still require Esplora or Electrum.

The backend CLI now accepts the common backend-specific knobs directly:

- `--insecure` for Electrum TLS bypass testing against servers you control
- `--cookiefile` or `--username` / `--password` for Bitcoin Core RPC auth
- `--wallet-prefix` for Bitcoin Core watch-only wallet naming

## Notes by backend type

### Esplora

Use this for mempool-compatible HTTP APIs.

- good for address and descriptor sync
- leaks queried scripts to the remote server
- easiest option when you are not running your own node

### Electrum

Use this for Electrum/Fulcrum-style servers.

- Kassiber uses scripthash calls and raw transaction fetches
- works for Bitcoin and for the current bundled Liquid endpoint
- `INSECURE=1` disables TLS verification and should only be used against servers you control

### Bitcoin Core RPC

Use this when you run your own node.

- today this path is for Bitcoin address-based sync; descriptor/xpub live sync is not implemented on `bitcoinrpc` yet
- Kassiber creates or reuses a dedicated watch-only Core wallet per Kassiber wallet
- this keeps wallet sync isolated instead of mixing unrelated watch-only imports together
- plain `http://` is only safe on localhost or over a trusted tunnel

## Descriptor and Liquid notes

Descriptor wallets derive receive and change scripts locally and then sync through an Esplora- or Electrum-backed backend.

Example Bitcoin descriptor wallet:

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

Example Liquid descriptor wallet:

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

- private SLIP77 blinding keys are required for full sync and fee accounting
- Kassiber accepts modern `ct(...)` and `elwpkh(...)` syntax and normalizes it internally
- the bundled `liquid` backend is still a third-party server from your machine's perspective

## Security reminders

- public backends learn your queried scripts and timing
- descriptor sync leaks more wallet structure than fixed-address sync
- `tor_proxy` is stored but not wired yet; route the whole process externally if needed
- backend credentials in CLI flags can land in shell history
- `backends get` / `list` are safe-to-record only for secret-bearing config values; other metadata may still be sensitive

See [SECURITY.md](../../SECURITY.md) for the current privacy model and outbound request inventory.
