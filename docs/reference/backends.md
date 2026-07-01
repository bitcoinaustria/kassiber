# Backends Reference

Kassiber syncs wallets through named backends. A backend is a local pointer to
an external indexer, node, or BTCPay instance that Kassiber uses to discover
transactions and balances.

Backends are stored canonically in SQLite.

- `~/.kassiber/config/backends.env` or your chosen `--env-file` is still
  accepted as a bootstrap / compatibility input for non-secret addressing
  fields (`KIND`, `URL`, `CHAIN`, `NETWORK`, `BATCH_SIZE`, `TIMEOUT`,
  `INSECURE`, `WALLETPREFIX`, `COOKIEFILE`, `KASSIBER_DEFAULT_BACKEND`)
- the `backends` table in SQLite is the long-term source of truth, and
  it is the only place secret-bearing fields (`TOKEN`, `PASSWORD`,
  `USERNAME`, `AUTH_HEADER`, plus the RPC aliases `RPCUSER` /
  `RPCPASSWORD`) should live once `kassiber secrets init` has put the
  database under SQLCipher
- native OS credential stores are not used for backend secrets in the current
  desktop secret-management slice; the AI-provider-key pilot is deliberately
  narrow, and backend tokens/auth headers/cookies/basic-auth remain
  SQLCipher-protected

Built-in defaults and dotenv-defined backends are imported into SQLite during
explicit bootstrap-import flows such as `kassiber init` or backend mutation
commands that need a canonical SQLite row. Read-only commands keep that
bootstrap config in memory only. Environment-only overrides stay ephemeral
unless you explicitly create the backend through the CLI.

## Built-in defaults

Without any user configuration, Kassiber currently ships these built-in names:

- `fulcrum` -> `electrum` -> `ssl://index.bitcoin-austria.at:50002`
- `mempool` -> `esplora` -> `https://mempool.bitcoin-austria.at/api`
- `liquid` -> `electrum` -> `ssl://les.bullbitcoin.com:995`
- `liquid-blockstream` -> `electrum` -> `ssl://blockstream.info:995`

`fulcrum` is the default for Bitcoin wallet sync. `mempool` remains the
built-in Esplora backend and the public explorer-link fallback. `liquid`
is the preferred built-in Liquid sync backend, while `liquid-blockstream`
is available as an alternate public Liquid Electrum endpoint.

## Useful commands

Inspect the merged backend view:

```bash
python3 -m kassiber backends list
python3 -m kassiber backends get mempool
```

Those inspection commands follow Kassiber's safe-to-record contract for
secret-bearing values: backend inspection returns an allowlisted safe view,
raw credentials and unknown config keys are suppressed, and credential
presence is exposed through `has_*` flags instead. If a backend URL contains
embedded credentials or query tokens, the displayed URL is sanitized before
it is emitted.

Create and manage SQLite-backed backends:

```bash
python3 -m kassiber backends create myelectrum --kind electrum --url ssl://index.bitcoin-austria.at:50002
python3 -m kassiber backends update myelectrum --display-name "Home Fulcrum"
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
  --backend fulcrum \
  --address bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq
```

## Dotenv layout

The key pattern is:

- `KASSIBER_DEFAULT_BACKEND`
- `KASSIBER_BACKEND_<NAME>_<FIELD>`

Example:

```dotenv
KASSIBER_DEFAULT_BACKEND=fulcrum

KASSIBER_BACKEND_FULCRUM_KIND=electrum
KASSIBER_BACKEND_FULCRUM_CHAIN=bitcoin
KASSIBER_BACKEND_FULCRUM_NETWORK=main
KASSIBER_BACKEND_FULCRUM_URL=ssl://index.bitcoin-austria.at:50002

KASSIBER_BACKEND_MEMPOOL_KIND=esplora
KASSIBER_BACKEND_MEMPOOL_CHAIN=bitcoin
KASSIBER_BACKEND_MEMPOOL_NETWORK=main
KASSIBER_BACKEND_MEMPOOL_URL=https://mempool.bitcoin-austria.at/api

KASSIBER_BACKEND_LIQUID_KIND=electrum
KASSIBER_BACKEND_LIQUID_CHAIN=liquid
KASSIBER_BACKEND_LIQUID_NETWORK=liquidv1
KASSIBER_BACKEND_LIQUID_URL=ssl://les.bullbitcoin.com:995

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
- `backends delete` removes that backend from wallet config; affected wallets cannot sync again until you assign another backend
- `--display-name` changes the user-facing label without changing the stable backend name that wallets reference
- process-level `KASSIBER_BACKEND_*` overrides still win for the current process even when a backend has already been imported into SQLite
- config-backed auth fields can be scrubbed with `backends update --clear ...`; clearing removes the stored key from SQLite instead of leaving the old value behind

## Supported backend kinds

Current backend kinds:

- `mempool`
- `esplora`
- `electrum`
- `bitcoinrpc`
- `btcpay`
- `lnd`
- `coreln`
- `liquid-esplora`
- `custom`

`liquid-esplora` remains supported for explicit Explorer API backends, but
the bundled Liquid defaults use Electrum because Liquid history refresh is
faster through those servers.

Common fields:

- `KIND`
- `URL`
- `TIMEOUT`
- `CHAIN`
- `NETWORK`
- `DISPLAY_NAME`

Electrum-specific fields:

- `BATCH_SIZE`
- `INSECURE`

Bitcoin Core-specific fields:

- `USERNAME`
- `PASSWORD`
- `COOKIEFILE`
- `WALLETPREFIX`

BTCPay-specific fields:

- `TOKEN`

LND-specific fields:

- `TOKEN` stores the read-only macaroon as hex
- `CERTIFICATE` stores either a path to `tls.cert` or PEM contents
- `INSECURE` is available for local/self-signed testing when you deliberately
  trust the endpoint

Create a read-only LND backend without putting the macaroon in shell history:

```bash
xxd -p -c 256 readonly.macaroon | \
  python3 -m kassiber backends create lnd --kind lnd \
    --url https://127.0.0.1:8080 \
    --certificate ~/.lnd/tls.cert \
    --token-stdin
```

Core Lightning-specific fields:

- `TOKEN` for a commando rune when using the least-privilege remote path
- `COMMANDO_PEER_ID`
- `LIGHTNING_CLI`
- `LIGHTNING_DIR`
- `RPC_FILE`

Both Lightning adapters expose the desktop node snapshot via
`ui.connections.node.snapshot` and the profitability summary via
`ui.reports.lightning_profitability`, dispatching through the shared
Lightning adapter registry. The adapters are strictly read-only: they
call `getinfo`, channel and balance endpoints, forwarding history,
payments, invoices, and the fee report, but never open, close, or pay.
Adapters drop preimages, encoded bolt11 strings, onion route hops,
route hints, and `failure_source_pubkey` before any payload reaches the
local DB. Private channels surface with `peer_pubkey=null` by default.

BTCPay backends now serve two separate Kassiber flows:

- wallet-history sync imports confirmed on-chain rows into configured wallets
  and keeps them as conservative transport transactions
- merchant provenance sync (`btcpay provenance sync`) reads invoice/payment
  records into separate provenance tables, preserving stable invoice/payment ids
  and raw payload snapshots without duplicating wallet balances

Use a Greenfield API key with wallet-history permissions for
`wallets sync-btcpay` and invoice-view permissions for
`btcpay provenance sync`. The review command is the gate that turns a matched
BTCPay payment into authoritative `btcpay_payment` pricing or a commercial
transaction kind.

Note: `bitcoinrpc` support is currently partial. Kassiber can use it for
Bitcoin address-based wallets, including the read-only UTXO inventory,
but descriptor- and xpub-backed source refresh still require Esplora or
Electrum.

The backend CLI now accepts the common backend-specific knobs directly:

- `--insecure` for Electrum TLS bypass testing against servers you control
- `--cookiefile` or `--username` / `--password` for Bitcoin Core RPC auth
- `--wallet-prefix` for Bitcoin Core watch-only wallet naming
- `--lightning-cli`, `--lightning-dir`, `--rpc-file`, and
  `--commando-peer-id` for Core Lightning

### Core Lightning read-only sync

Core Lightning node sync is intentionally read-only from Kassiber's side.
The adapter only calls `getinfo`, `bkpr-list*`, and a curated subset of
`list*` RPC methods; it never calls payment, invoice creation, channel
mutation, wallet mutation, or signing methods, and the allowlist in
`CLN_ALLOWED_METHODS` rejects any unsupported method at the transport
boundary even if the rune would permit it.

Preferred least-privilege setup is a commando rune restricted to read
and bookkeeper methods with a rate cap:

```bash
lightning-cli commando-rune restrictions='[["method^list","method^get","method^bkpr-list","method=summary"],["method/listdatastore"],["rate=60"]]'
```

Store that rune through stdin or an fd so it does not land in shell
history. Kassiber passes the rune through the `LIGHTNING_RUNE`
environment variable when invoking `lightning-cli`, so it never appears
in `/proc/<pid>/cmdline`:

```bash
printf %s "$CLN_READONLY_RUNE" | python3 -m kassiber backends create cln \
  --kind coreln \
  --url cln://commando \
  --commando-peer-id <node-id> \
  --token-stdin

python3 -m kassiber wallets create \
  --label routing-node \
  --kind coreln \
  --backend cln

python3 -m kassiber wallets sync --wallet routing-node
python3 -m kassiber reports lightning-profitability --connection routing-node
python3 -m kassiber reports export-lightning-profitability-csv \
  --connection routing-node \
  --file /tmp/kassiber-lightning-profitability.csv
```

In the desktop app, add Core Lightning from Settings -> Sync backends.
The Core Lightning form stores the backend and creates the matching
read-only node connection so normal wallet sync can refresh it.

Local RPC-file use is also supported for operators who run Kassiber on
the same machine as `lightningd`, but local RPC access is not
least-privilege on its own. Prefer the commando rune path when you want
the connection itself to be unable to pay, create invoices, close
channels, or mutate wallet state.

Persisted CLN records follow the discard policy in
[lightning-opsec.md](lightning-opsec.md): forwards are aggregated to
day-per-channel rows (no per-forward log of "X paid Y through me"),
balance snapshots are daily-bucketed (no fresh row per sync), invoice
events from `bkpr-listincome` become wallet transactions (routed events
do not, avoiding the double-count with the per-forward aggregate), and
no raw RPC payloads are stored on disk.

### BTCPay Greenfield API

Use this to pull confirmed on-chain wallet transactions directly from a BTCPay server instead of exporting CSV or JSON from the UI.

- create a backend with `--kind btcpay`, `--url https://btcpay.example.com`,
  and a piped `--token-stdin` (preferred) or `--token-fd FD` for the
  Greenfield API key — the argv form `--token <value>` still works for
  legacy scripts but emits a deprecation warning and leaks to shell history
- store the BTCPay wallet config on the wallet with `wallets create/update --backend <btcpay-backend> --store-id <store-id>`
- `wallets sync-btcpay --wallet <label> --backend <btcpay-backend> --store-id <store-id>` keeps the legacy one-off CLI shape and now stores that config on the wallet too
- the desktop Add Connection dialog can create the BTCPay instance inline from
  URL + API key, discover stores/payment methods, and then either create one
  BTCPay-backed wallet source per selected sync-supported payment method or map
  those payment methods onto existing settlement wallets without sending the
  user through backend settings first
- once the config is stored, `wallets sync --wallet <label>` and `wallets sync --all` reuse it automatically
- use one Kassiber wallet per real underlying wallet / BTCPay-backed balance source; if multiple BTCPay stores point at the same underlying wallet balance, keep them on one Kassiber wallet or holdings will be duplicated
- when a Liquid or multisig settlement wallet is already configured elsewhere,
  store BTCPay as provenance on that wallet instead of adding a second wallet
  source for the same balance — use `wallets attach-btcpay --wallet <label>
  --backend <btcpay-backend> --store-id <store-id>` from the CLI, or the
  desktop Add Connection "Map existing wallets" mode
- Kassiber requests confirmed rows only, then normalizes them through the existing BTCPay import pipeline so comments become notes and labels become tags
- the Greenfield wallet-transaction endpoint currently requires the `btcpay.store.canmodifystoresettings` permission on the API key

## Notes by backend type

### BTCPay

Use this when a BTCPay store is the authoritative transaction source for a real wallet balance, or when BTCPay should enrich existing settlement wallets with store-side payment metadata.

- best fit for merchant stores where BTCPay comments/labels are part of the local bookkeeping story
- current refresh is confirmed-only and reuses the BTCPay import pipeline so comments become notes and labels become tags
- BTCPay-only mode is enough when BTCPay has all relevant store wallet history; existing-wallet mode is better when on-chain or Liquid wallets are already tracked separately
- this is not full invoice/payment provenance yet; stable invoice ids and raw payload snapshots are still later work

### Esplora

Use this for mempool-compatible HTTP APIs.

- good for address and descriptor refresh
- leaks queried scripts to the remote server
- easiest option when you are not running your own node

### Electrum

Use this for Electrum/Fulcrum-style servers.

- Kassiber uses scripthash calls and raw transaction fetches
- works for Bitcoin and for the current bundled Liquid endpoint
- accepts clearnet hosts and `.onion` hosts, for example
  `tcp://abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcd.onion:50001`
  or `ssl://...onion:50002` when the server's TLS setup matches
- `INSECURE=1` disables TLS verification and should only be used against servers you control

### Tor / `.onion` backends

Supported backend URLs may be clearnet or `.onion`. For example:

```bash
python3 -m kassiber backends create fulcrum-onion \
  --kind electrum \
  --url tcp://abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcd.onion:50001 \
  --tor-proxy 127.0.0.1:9050

python3 -m kassiber backends create esplora-onion \
  --kind esplora \
  --url http://abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcd.onion/api \
  --tor-proxy 127.0.0.1:9050
```

In the desktop app, entering a `.onion` backend host prefills the standard
local Tor SOCKS proxy for that backend only. Kassiber does not start Tor; keep
your Tor service running separately.

### Bitcoin Core RPC

Use this when you run your own node.

- today this path is for Bitcoin address-based refresh; descriptor/xpub refresh is not implemented on `bitcoinrpc` yet
- Kassiber creates or reuses a dedicated watch-only Core wallet per Kassiber wallet
- this keeps refresh state isolated instead of mixing unrelated watch-only imports together
- plain `http://` is only safe on localhost or over a trusted tunnel

## Descriptor and Liquid notes

Descriptor wallets derive receive and change scripts locally and then refresh
through an Esplora- or Electrum-backed backend. They accept output descriptors,
common descriptor exports, and plaintext BSMS descriptor records. Source refresh
also updates the durable local UTXO inventory shown in the desktop wallet
detail view.
The default gap limit is 40 unused addresses per branch, and Kassiber caps the configured gap limit at 5,000 to avoid accidental runaway scans.

Example Bitcoin descriptor wallet:

```bash
bash -c 'python3 -m kassiber wallets create \
  --label vault \
  --kind descriptor \
  --backend mempool \
  --descriptor-fd 3 \
  --change-descriptor-fd 4 \
  --gap-limit 40' \
  3< <(printf '%s\n' 'wpkh([fingerprint/84h/0h/0h]xpub.../0/*)') \
  4< <(printf '%s\n' 'wpkh([fingerprint/84h/0h/0h]xpub.../1/*)')

python3 -m kassiber wallets derive --wallet vault --count 5
python3 -m kassiber wallets sync --wallet vault
```

Example Liquid descriptor wallet:

```bash
bash -c 'python3 -m kassiber wallets create \
  --label event-liquid \
  --kind descriptor \
  --backend liquid \
  --chain liquid \
  --network liquidv1 \
  --descriptor-fd 3 \
  --change-descriptor-fd 4 \
  --gap-limit 40' \
  3< <(printf '%s\n' 'ct(slip77(...),elwpkh(.../0/*))') \
  4< <(printf '%s\n' 'ct(slip77(...),elwpkh(.../1/*))')
```

Liquid UTXO inventory is only populated when Kassiber can unblind the output
locally from descriptor material. If private blinding keys are missing, the
desktop UTXOs table shows a Liquid unblind blocker instead of guessing output
amounts or assets.

For Liquid:

- private SLIP77 blinding keys are required for full sync and fee accounting
- Kassiber accepts modern `ct(...)` and `elwpkh(...)` syntax and normalizes it internally
- the bundled `liquid` backend is still a third-party server from your machine's perspective

## Security reminders

- public backends learn your queried scripts and timing
- descriptor sync leaks more wallet structure than fixed-address sync
- descriptors and blinding keys are Kassiber-managed secrets; prefer stdin/fd
  entry over inline argv, and avoid temporary plaintext descriptor files
- `tor_proxy` is a deliberate per-backend routing choice. It is honored by
  Electrum, Esplora / Explorer-API HTTP reads, BTCPay Greenfield sync, Bitcoin
  Core RPC, and mempool-rate fetches that use a configured backend. Partial
  routing is supported: a proxy on one backend does not route any other
  backend, AI provider, or standalone rate provider. Values may be `HOST:PORT`,
  `socks5h://HOST:PORT`, `socks5h://USER:PASS@HOST:PORT`, or `http(s)://...`;
  percent-encode special username/password characters. It is not a bundled Tor
  daemon or a global proxy for standalone Coinbase/CoinGecko rate providers.
  The desktop setup forms detect `.onion` hosts, prefill `127.0.0.1:9050`, and
  keep the notice scoped to the backend being edited; Tor itself must already be
  running.
- credentials in argv (`--token <value>`, `--password <value>`,
  `--auth-header <value>`, `--username <value>`) land in shell history
  and the process listing — use the `--*-stdin` / `--*-fd FD` variants
  instead; argv forms warn but still work for legacy scripts
- after `kassiber secrets init`, secrets do not belong in the plaintext
  `backends.env` bootstrap; lift any pre-existing entries into the
  encrypted `backends` table with `kassiber secrets migrate-credentials`
  (URLs and other addressing fields stay in the dotenv)
- `backends get` / `list` are safe-to-record only for secret-bearing config values; other metadata may still be sensitive

See [SECURITY.md](../../SECURITY.md) for the current privacy model and outbound request inventory.
