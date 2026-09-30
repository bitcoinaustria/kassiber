# Backends Reference

Kassiber syncs wallets through named backends. A backend is a local pointer to
an external indexer, node, or BTCPay instance that Kassiber uses to discover
transactions and balances.

Backends are stored canonically in SQLite.

- `<state-root>/projects/<project>/config/backends.env` or your chosen
  `--env-file` is still accepted as a project-local bootstrap / compatibility
  input for non-secret addressing fields (`KIND`, `URL`, `CHAIN`, `NETWORK`,
  `BATCH_SIZE`, `TIMEOUT`, `INSECURE`, `CERTIFICATE`, `WALLETPREFIX`, `COOKIEFILE`,
  `KASSIBER_DEFAULT_BACKEND`)
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

Desktop setup offers three choices:

- **Built-in public backends** enables the four presets above.
- **Custom sync backend** saves only the backend entered during setup.
- **Work offline** starts with no backends and no public explorer links.

Custom and offline books stay in manual mode. Adding a backend later in
Settings does not restore the public presets. The first compatible Bitcoin
sync backend added to an offline book becomes its default.

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
- deleting a bootstrap-backed backend suppresses the built-in/default bootstrap copy; backends forced by `backends.env` or process variables must be removed there and the process restarted first
- deleting the active stored default atomically selects a remaining backend on the same chain that wallets can actually sync against, preferring a user-created one; manual mode may return to zero backends, while built-in mode keeps at least one
- `backends delete` refuses backends still referenced by wallets; reassign those wallets first
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

Supported watch-only Bitcoin descriptors on Esplora/Electrum are observed by
BDK 3.0.0; supported confidential Liquid descriptors are observed by LWK
0.18.0. Selection is capability-based and happens before connection. Backends
requiring transport features the pinned binding cannot express (for example a
Liquid SOCKS proxy, custom CA, Esplora authorization, or non-default Electrum
timeout) use an explicitly named compatibility observer. A dependency runtime
failure never retries through that path. The complete matrix and removal
conditions are in [Chain observers](chain-observers.md).

Observer state is derived and versioned. An ordinary refresh fails closed with
`observer_state_rebuild_required` when it encounters an unknown/newer state,
coverage, or LWK opaque-value namespace. A forced/full refresh reconstructs the
dependency state in memory and replaces it only in the successful atomic apply;
it does not pre-delete transactions, notes, attachments, review history, or
custody interpretations.

An ordinary `wallets sync` reuses the wallet's persisted freshness checkpoint:
scripts whose backend status is unchanged since the last sync are skipped and
their records are not re-emitted. That means a plain resync will not restore
transaction rows that were removed locally while the chain view stayed
unchanged. `wallets sync --force-full` ignores the stored checkpoint and
replays the full backend history; it is the repair path whenever local rows
may have diverged from an unchanged-chain checkpoint (the desktop app's
forced-full refresh is the same path).

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

Bitcoin transactions synced through an Electrum backend persist their decoded
graph in `transactions.raw_json` with inline prevouts (Esplora-shaped
`scriptpubkey`/`value` outputs) plus a `_kassiber_electrum_graph` marker object
recording the shape kind and version. The marker lets unchanged-chain resyncs
reuse the stored graph instead of re-fetching transaction bodies; external
tooling that parses `raw_json` should ignore the marker key.

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

BTCPay backends serve two separate Kassiber flows, described in
[BTCPay Server](#btcpay-server-greenfield-api): wallet-history import for store
wallets and merchant provenance (invoices, payments, refunds, payouts) that
explains transactions without adding balances.

`bitcoinrpc` supports Bitcoin descriptor/xpub/address wallet refresh. For
descriptor/xpub wallets Kassiber imports ranged watch-only descriptors into a
dedicated Core wallet; that import is a backend mutation and may trigger Core's
blocking rescan. Use the wallet's real `--birthday` date to bound that rescan.
For a pruned node, a birthday is required before Kassiber imports new watch
targets or starts a full rescan. The probe reports Core's approximate earliest
retained date and the two-hour timestamp safety window, but it does not guess a
rescan height from raw block timestamps: those timestamps are not monotonic and
Core's cumulative max-time index is not exposed over RPC. The actual
`importdescriptors` / `importmulti` rescan is authoritative. If Core reports
that required blocks are unavailable, the operation fails closed and directs
the user to an unpruned Core node or archival backend. Do not move the birthday
forward merely to bypass that result, because doing so can silently omit older
wallet activity.

Kassiber writes a local history attestation only after Core successfully
completes that import/rescan. Core watch membership alone is not enough: an
import can leave a target watched even if its rescan failed. A missing or stale
attestation therefore forces a re-import, including for already-watched address
wallets. Once the attestation matches the backend, Core wallet, birthday, and
target set, ordinary incremental refresh can continue without rescanning. A
full rescan clears that fast path; descriptor range expansion, birthday/target
changes, or recreation of Core's Kassiber watch wallet also require a new
successful attestation. The desktop setup shows the retained horizon, requires
a birthday when scanning a pruned node, and makes clear that Core verifies
coverage during the scan. Creating the Kassiber wallet without scanning remains
available.

The desktop Core detector reads default cookie locations and local
`bitcoin.conf` RPC settings, then probes reachability, peer/sync state, wallet
RPC support, and BIP158 block-filter availability. Block filters are reported
for operator visibility; descriptor sync does not require `blockfilterindex=1`.

The backend CLI now accepts the common backend-specific knobs directly:

- `--insecure` for Electrum TLS bypass testing against servers you control
- `--certificate` for a verified custom CA bundle on HTTPS/Electrum backends;
  LND also accepts its `tls.cert` path or PEM contents
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

### BTCPay Server (Greenfield API)

Kassiber reads BTCPay Server through read-only Greenfield `GET` requests. It
never creates invoices or payouts, changes a store, or reads payment-method
configuration (derivation schemes and Lightning connection strings stay in
BTCPay). Each saved `btcpay` backend is one server URL plus one API key.

#### 1. Create an API key

| Preset | Permission | What Kassiber can do |
| --- | --- | --- |
| `read_only` | `btcpay.store.canviewstoresettings` | Stores, enabled payment methods, invoices and payments, payment requests, refunds and payouts, and the on-chain address preview used to recognise which wallet a store pays into. Enough for Lightning and plugin payment ledgers and for stores whose wallet you already track. The key cannot change the store. |
| `wallet_history` | `btcpay.store.canmodifystoresettings` | Everything above plus each store's on-chain wallet as BTCPay records it, with labels, comments, and payout fees. BTCPay exposes wallet history only under this permission, which can also change the store, including where it receives payments. |

`kassiber btcpay key-url --server-url https://btcpay.example.com [--preset
read_only] [--scope all|single] [--store-id ID]` prints the permissions, manual
steps, and a pre-filled `/api-keys/authorize` link; it makes no network
request. The desktop **Open BTCPay to create this key** button opens the same
link in your browser, where you sign in and approve; BTCPay then shows the new
key on its API Keys page. Paste it into the desktop form, or pipe it into
`backends create --kind btcpay --token-stdin`.

BTCPay's authorize page grants a store permission either on all stores or on
exactly one store it asks you to pick. `--scope all` (the default for
`read_only`) gives one key for every store, including stores added later;
`--scope single` (the default for `wallet_history`) limits the key to one
store. `--store-id` pins the permission to named stores.
URLs pasted from the browser are normalised: a trailing `/api/v1`, store page,
or slash is removed.

#### 2. Inspect the key and stores

`kassiber btcpay inspect --backend NAME` (desktop: **Check key and stores**)
reads the server version and sync state, the key's own permissions, every
visible store, its enabled payment methods, and the first receive addresses of
each on-chain store wallet. It reports:

- the key's scope (all stores or selected stores) and risk
  (`read_only`, `can_write`, `can_modify_store`, `server_admin`), plus any
  permission broader than Kassiber needs;
- per store, which capabilities the key grants and which permissions are
  missing;
- per payment method, its rail and a recommendation, see below;
- stores that pay into the same wallet (same address preview), existing
  wallets Kassiber already tracks for a store (local ownership match of the
  preview addresses), other saved keys for the same server, the detected
  network, and transport warnings for plain `http://`.

The preview addresses are only used in memory; the plan carries an opaque
`wallet_fingerprint`.

#### 3. Choose what Kassiber does per payment method

| Action | Use it for |
| --- | --- |
| `wallet_source` | Import BTCPay's confirmed on-chain wallet history as a Kassiber wallet. Needs the `wallet_history` key. Comments become notes, labels become tags, and payout fees are booked as fees. |
| `existing_wallet` | The store pays into a wallet you already track (descriptor, xpub, Liquid). The wallet keeps its own chain sync; BTCPay adds invoice provenance and, with a history key, labels and comments. Recommended automatically when the preview addresses belong to that wallet. |
| `payment_ledger` | The store settles into something Kassiber cannot watch: a Lightning node or wallet, a plugin rail, or a layer-two wallet. BTCPay's settled payments and completed payouts become the wallet's balance. Recommended for Lightning when no Lightning node is connected to the book, and for bitcoin plugin rails. |
| `provenance_only` | Keep invoices, payments, and payouts only. Used when a connected Lightning node already books the payments, for a store that shares a wallet already imported through another store, or with a read-only key for an on-chain wallet Kassiber does not track yet. |
| `skip` | Ignore, or remove a previous route. Non-bitcoin assets are skipped. |

`kassiber btcpay setup --backend NAME --label LABEL --recommended` applies the
suggested plan; `--route STORE[:METHOD]=ACTION[@WALLET]` overrides single
payment methods and `--dry-run` shows the routes without writing. The desktop
applies the same plan through `ui.connections.btcpay.create`. The core refuses
plans that would import one store wallet twice or book one store's Lightning
payments through two keys. For an on-chain store with a read-only key, create a
wallet-history key to import the wallet from BTCPay, or map the store to a
wallet you already track.

#### Multiple stores and keys

- One key can cover many stores; discovery lists every store it can see.
- A second key for the same server (for example one per store owner, or a
  wallet-history key scoped to one store) is saved as another backend. The plan
  lists `sibling_backends` and flags stores already imported through another
  key, so a store wallet is never imported twice.
- Stores sharing one wallet are imported once: the first store (by name) takes
  the wallet source or mapping and the others keep provenance, or all of them
  map onto the same tracked wallet.
- Rotate a key with `backends update NAME --token-stdin`; routes reference the
  backend name, and provenance records are keyed by store id, not by key.

#### Payment methods and plugins

| Payment method | Kassiber behaviour |
| --- | --- |
| `BTC-CHAIN`, `LBTC-CHAIN` | Wallet source, mapping, or provenance. BTCPay 1.x ids (`BTC`) are accepted. |
| `BTC-LN` | Payment ledger, or provenance when the node that receives the payments (LND, Core Lightning) is connected to the book; Kassiber then links BTCPay payments to it by payment hash. Plugin wallets behind `BTC-LN` (Boltz → Liquid, Breez, Blink, Strike, Nostr Wallet Connect, …) usually cannot be watched, so the ledger is the balance source. |
| `BTC-LNURL` (LNURL, Lightning Address) | Same as `BTC-LN`, booked in the same Lightning ledger for the store. |
| Other `BTC-…` plugin rails | Payment ledger, one per rail. |
| Plugin rails in an unknown currency, altcoins, and Liquid assets (`ARKADE`, `XMR-CHAIN`, `USDT-CHAIN`, …) | Provenance or skipped; Kassiber books only payments it can confirm are bitcoin. |

#### Payment ledgers

A payment ledger is a wallet whose rows come from BTCPay's records: each
settled invoice payment is a receipt and each completed payout or refund a
send, identified by its provenance key so reruns update rows in place and
commercial links match exactly. BTCPay's payment is the income evidence;
review the suggested link to apply its invoice price.

- The ledger sees only what passes through BTCPay. Withdrawals from the node
  or wallet outside BTCPay (closing channels, sweeping to cold storage) need
  their own record, for example a transfer to the receiving wallet.
- Lightning routing fees of payouts are not reported by BTCPay; the payout
  amount is booked.
- A payment another connected wallet already books by payment hash in the
  same direction, such as the store's own LND node, is held back and reported
  as `held_tracked_elsewhere` instead of being counted twice. A payment from
  your own wallet into your store books outbound there and inbound in the
  ledger, which is a transfer, not a duplicate. Rows the ledger booked before
  such a wallet was connected are not removed automatically; the sync reports
  them as `existing_tracked_elsewhere` so you can exclude them, or archive the
  ledger wallet and keep the store's Lightning invoices as provenance.
- Pending payments are reported as `pending` and booked once they settle.
- A ledger sync fails when BTCPay cannot be read, instead of booking from old
  records.

#### What is synced

Wallet syncs of a BTCPay wallet source or mapped wallet also refresh that
store's invoices, payment requests, and payouts, then create local link
suggestions (`btcpay provenance links --state suggested`). `btcpay provenance
sync --all` refreshes every configured store. Invoice origins recognise BTCPay
Point of Sale and Crowdfund apps, payment requests (titles are loaded when an
invoice has no description), plugin app routes, and WooCommerce/Shopify orders.
Unpaid invoices are not hydrated, unchanged pages are skipped, and open
(New/Processing) invoices are re-checked until they settle. Invoices priced in
bitcoin (`BTC`, `SATS`, common for Point of Sale and crowdfund apps) carry no
fiat price: reviewing them sets the commercial kind and leaves the
transaction's pricing to market rates.

Payouts cover invoice refunds (pull payments named `Refund {invoiceId}`),
pull-payment claims, and store payouts created by plugins such as Prism or
payroll tools. A completed payout's proof carries the txid or Lightning payment
hash; Kassiber suggests it only against an outbound transaction and reviews it
as `refund` or `expense`, never income. Reviewing applies the payout's fiat
value as `btcpay_payout` pricing. Lightning preimages and encoded BOLT11/LNURL
destinations are not stored.

A send that pays several payouts at once is reviewed as one batch: confirming
any of its payouts confirms all of them, prices the transaction at their summed
fiat value (`payout_batch` granularity), and reopening any of them restores the
transaction. Payouts in different fiat currencies are refused as a batch.

#### Fees

BTCPay's wallet history reports each send as the net wallet change with the
miner fee folded in. When completed payouts explain a send, the difference is
booked as the fee (principal = the payouts, fee = the rest) on the next wallet
sync, including sends imported before their payouts completed. A remainder
above 0.002 BTC or at least half the send stays in the send, because it is
more likely an output Kassiber does not know about. Other sends keep BTCPay's
fee-inclusive amount, which is exact for balances.

#### Freshness

Kassiber does not receive BTCPay webhooks or any other push updates. Store data
is as current as the last sync, which is either explicit (`wallets sync`,
`btcpay provenance sync --all`, the desktop refresh) or background freshness
once the BTCPay source class is enabled. Every store refresh records its last
attempt, last success, and last error locally (`btcpay_store_sync_states`, not
replicated). `btcpay inspect` reports each configured store's `sync_state` and
a `stale_store_data` warning when a store was not refreshed for a day or its
last refresh failed; the transaction commercial panel shows the same age.

## Notes by backend type

### BTCPay

Use BTCPay when a store's invoices, payments, refunds, and payouts should
explain your wallet transactions, or when BTCPay is the only record of a store
wallet's history.

- prefer a read-only key plus a watch-only descriptor wallet for each store
  wallet; the wallet-history key trades safety for BTCPay's own labels
- one Kassiber wallet per real wallet: stores sharing a wallet map onto it once
- Lightning, LNURL, and plugin rails stay provenance; track the node or wallet
  that receives the funds as its own connection
- see [BTCPay Server](#btcpay-server-greenfield-api) for setup and limits

### Esplora

Use this for mempool-compatible HTTP APIs.

- good for address and descriptor refresh
- leaks queried scripts to the remote server
- easiest option when you are not running your own node
- an HTTPS endpoint whose certificate chains to a root outside the public web
  PKI — a private or organizational CA, even one already installed in your system
  trust store — needs `CERTIFICATE` pointed at that root (or at the host bundle
  itself). The native Rustls-based observer ships its own root set and does not
  read the system store, so setting `CERTIFICATE` is also what moves this one
  backend onto the transport that enforces it

### Electrum

Use this for Electrum/Fulcrum-style servers.

- Kassiber uses scripthash calls and raw transaction fetches
- works for Bitcoin and for the current bundled Liquid endpoint
- accepts clearnet hosts and `.onion` hosts, for example
  `tcp://abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcd.onion:50001`
  or `ssl://...onion:50002` when the server's TLS setup matches
- `CERTIFICATE=/path/to/ca.pem` uses that verified CA bundle as this backend's
  trust store
- `INSECURE=1` disables all certificate and hostname verification; it does not
  pin a self-signed certificate and should only be used against servers you control

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

- supports Bitcoin descriptor/xpub/address refresh from your own node
- Kassiber creates or reuses a dedicated watch-only Core wallet per Kassiber wallet
- descriptor imports are ranged per receive/change branch; repeated refreshes
  widen the range from observed transaction history / UTXOs and otherwise use
  `listsinceblock`
- local desktop detection understands default cookie files plus `bitcoin.conf`
  RPC auth/ports, and the health probe reports peers, sync state, wallet RPC
  support, pruning, IBD, and BIP158 filter-index availability
- this keeps refresh state isolated instead of mixing unrelated watch-only imports together
- plain `http://` is only safe on localhost or over a trusted tunnel
- HTTPS uses the host trust store by default; `CERTIFICATE` selects a custom CA
  bundle, while `INSECURE=1` disables verification for that backend only

## Descriptor and Liquid notes

Descriptor wallets derive receive and change scripts locally and then refresh
through an Esplora- or Electrum-backed backend. They accept output descriptors,
common descriptor exports, and plaintext BSMS descriptor records. Source refresh
also updates the durable local UTXO inventory shown in the desktop wallet
detail view.
The default gap limit is 100 unused addresses per branch, and Kassiber caps the
configured gap limit at 5,000 to avoid accidental runaway scans. If a
backend-synced wallet's active transaction history ever produces a negative
running balance, Kassiber runs one full repair refresh; descriptor/xpub wallets
use a temporary widened gap limit for that repair so missed high-index receive
or change addresses can be discovered without permanently changing the stored
wallet config.

Example Bitcoin descriptor wallet:

```bash
bash -c 'python3 -m kassiber wallets create \
  --label vault \
  --kind descriptor \
  --backend mempool \
  --descriptor-fd 3 \
  --change-descriptor-fd 4 \
  --gap-limit 100' \
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
  --gap-limit 100' \
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

See [Privacy & security](privacy-and-security.md) for the current privacy model and outbound request inventory.
