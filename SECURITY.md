# Security & Privacy

Kassiber is local-first. The database and all computation stay on your
machine. There is no telemetry, crash reporter, update check, license
check, or analytics. Outbound traffic is limited to the requests listed
below.

Kassiber is pre-release (`0.1.x`) — treat this as a description of
current behavior, not a long-term contract.

## The big gotcha: not running your own node

Out of the box, Kassiber ships three built-in named backends:

- `mempool` → `esplora` → `https://mempool.bitcoin-austria.at/api` — the
  default for Bitcoin wallets, operated by Bitcoin Austria.
- `fulcrum` → `electrum` → `ssl://index.bitcoin-austria.at:50002` —
  Bitcoin-Austria-operated Electrum/Fulcrum indexer.
- `liquid` → `electrum` → `ssl://les.bullbitcoin.com:995` — a
  third-party Liquid Electrum endpoint operated by BullBitcoin.

Every `wallets sync` against any of these sends your addresses (or
scripthashes, or gap-limit-scanned descriptor scripts) to whoever
operates that endpoint. They can link the queries to your IP and the
identifying `User-Agent: kassiber/<version>` header. "Bitcoin Austria
operates it" is still third-party from *your* machine's point of view;
"BullBitcoin operates it" for Liquid especially so.

Descriptor wallets are worse than address wallets here: gap-limit
discovery leaks a contiguous run of receive + change scripts, so the
backend sees the wallet cluster rather than just individual addresses.

Mitigations, in order of effect:

1. Run your own Bitcoin Core and use a `bitcoinrpc` backend (traffic
   stays on-box).
2. Run your own Esplora / Electrs / Fulcrum and use it as an `esplora`
   or `electrum` backend.
3. Torify the process (`torsocks python3 -m kassiber ...`) or route
   through a VPN. Kassiber has no built-in SOCKS support yet.
4. Prefer `address`-kind wallets over `descriptor`-kind wallets when
   you only care about a fixed set of addresses.
5. Skip `rates sync` and use `rates set` for manual rate upserts.

## External requests (complete list)

All HTTP(S) requests send `User-Agent: kassiber/<version>`. This is not
configurable.

| Trigger | Destination | Transport | What the other side learns |
| --- | --- | --- | --- |
| `wallets sync` against the built-in `mempool` default | `https://mempool.bitcoin-austria.at/api` (Bitcoin Austria) | Esplora over HTTPS | IP, User-Agent, scripthashes, query timing, descriptor scan shape |
| `wallets sync` against the built-in `fulcrum` default | `ssl://index.bitcoin-austria.at:50002` (Bitcoin Austria) | Electrum JSON-RPC over TLS | IP, queried scripthashes, query timing |
| `wallets sync` against the built-in `liquid` default | `ssl://les.bullbitcoin.com:995` (BullBitcoin) | Electrum JSON-RPC over TLS | IP, queried Liquid scripthashes, query timing |
| `wallets sync` against a user-configured Esplora backend | your configured URL | Esplora over HTTP(S) | same categories as `mempool` above |
| `wallets sync` against a user-configured Electrum backend | your configured `ssl://` or `tcp://` URL | Electrum JSON-RPC over raw TCP/TLS | IP, queried scripthashes, query timing |
| `wallets sync` against a `bitcoinrpc` backend | your configured URL | HTTP(S) POST with Basic auth | nothing leaves your machine if the node is local |
| `rates sync` (only) | `https://api.coingecko.com/api/v3/coins/bitcoin/market_chart` | unauthenticated HTTPS GET | IP, User-Agent, which fiat pair and window |

Nothing else makes network calls. `rates set`, `rates latest`,
`rates range`, `rates pairs`, journal processing, metadata CRUD, and all
reports are fully offline.

## Local storage

- `~/.kassiber/data/kassiber.sqlite3` — default SQLite DB. Contains
  descriptors, xpubs, addresses, transactions, metadata, rates cache,
  and backend overlay.
- `~/.kassiber/config/backends.env` — default backend config file. May contain
  Bitcoin Core RPC credentials and backend tokens.
- `~/.kassiber/config/settings.json` — managed state manifest for the active
  path layout. Not secret by itself, but it reveals where the rest of the
  local state lives.
- `~/.kassiber/attachments/` — managed attachment store for copied local files.
  URL attachments are stored as literal references in the database and are not
  fetched.
- Liquid descriptor wallets embed **private SLIP77 blinding keys** in
  `wallets.config_json`. Anyone who can read the DB can unblind your
  confidential outputs.
- Older installs may still resolve to `~/.local/share/kassiber`,
  `~/.local/share/satbooks`, or a legacy `<data-root>/.env`; run
  `kassiber status` to see the active paths.
- Keep backend config out of version control. Prefer `COOKIEFILE` over inline
  `USERNAME` / `PASSWORD`.

### At-rest encryption — not implemented yet

**Nothing Kassiber writes is encrypted.** The SQLite database, `backends.env`
file, and any exported CSVs are plain files on disk. That includes
every xpub, descriptor, SLIP77 blinding key, backend URL, auth header,
token, and RPC credential the tool has touched.

Today the only protection layer is whatever your OS gives you —
full-disk encryption, user account separation, file permissions.

The intended direction is seamless integration with the OS keychain
(macOS Keychain, Linux freedesktop secret-service / libsecret, Windows
DPAPI / Credential Manager) so that sensitive fields — blinding keys,
RPC credentials, backend tokens — are sealed by default and unlocked
on demand, without the user managing a separate passphrase. Until that
lands, treat the data directory and backend config file as sensitive material.

## Caveats

- **Secrets on the command line end up in shell history.** `backends
  create --token ...` and `--auth-header ...` write credentials into
  `~/.zsh_history` / `~/.bash_history`. Prefer `~/.kassiber/config/backends.env`
  (or another `--env-file`) or environment
  variables for anything sensitive.
- **`--debug` and `--machine` output can leak secrets.** Debug stack
  traces and machine envelopes include backend URLs, tokens, and —
  for Liquid descriptor wallets — private SLIP77 blinding keys. Redact
  before pasting into issues, screenshots, or logs.
- **Cross-wallet linkability.** Running `wallets sync` for several
  wallets in one session ties them to the same IP + timing +
  `User-Agent` at the backend. Per-wallet sync calls are not per-wallet
  privacy.
- **`tor_proxy` is scaffolded but not wired.** `backends create
  --tor-proxy` accepts a value and stores it, but HTTP and Electrum
  traffic currently ignores it. For now, torify the whole process
  externally.
- **No SPV / header verification.** Backends are trusted for transaction
  history, confirmations, and fees. A malicious backend can fabricate or
  hide transactions.
- **No rate-source cross-check.** Wrong CoinGecko rates become wrong
  cost basis becomes wrong capital-gains. For tax-grade numbers prefer
  `rates set` with values you trust.
- **Austrian tax support is still experimental.** Kassiber does have an
  Austrian `at` policy path today, but it remains review-gated, conservative,
  and in architectural transition toward the Kassiber-maintained RP2 fork.
  Neither generic nor Austrian output should be treated as jurisdiction-specific
  tax advice without review.
- **`Altbestand` is a bookkeeping assertion,** not a cryptographic
  proof. Keep your own paper trail.
- **Electrum `INSECURE=1` disables TLS verification.** Only against
  servers you fully control — never against a public Electrum server.
- **Plain HTTP to Bitcoin Core is only safe on localhost.** Kassiber
  will send RPC credentials over `http://` to whatever URL you
  configure. Tunnel remote nodes over SSH / VPN / TLS proxy.
- **Fixed, identifying `User-Agent`.** Every outbound HTTP request
  advertises `kassiber/<version>`.
- **Legacy data-root fallback.** If `~/.kassiber` does not exist yet but
  `~/.local/share/kassiber` or `~/.local/share/satbooks` does, Kassiber
  keeps using the older directory. `kassiber status` shows the effective
  path.
- **Declared-but-inactive wallet kinds.** `coreln`, `lnd`, `nwc` exist
  in the catalog but do not sync; credentials registered against them
  sit in the DB unused.

## Reporting

Do not file security-impacting issues in the public tracker. Contact
the maintainer privately with a reproduction.
