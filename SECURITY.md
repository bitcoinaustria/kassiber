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
  backend definitions/defaults, and any stored backend credentials.
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

### At-rest encryption — passphrase-gated SQLCipher (V4.1)

The SQLite database is now optionally encrypted via SQLCipher 4. After
running `kassiber secrets init`, every subsequent invocation needs a
passphrase: type it interactively, or pass `--db-passphrase-fd <FD>`
from a parent process.

- `~/.kassiber/data/kassiber.sqlite3` — when encrypted, contents are
  protected by SQLCipher 4 with stock PBKDF2-HMAC-SHA512
  (`kdf_iter = 256000`). Recoverable with the upstream `sqlcipher`
  binary using only the passphrase.
- The pre-migration plaintext file is preserved as
  `kassiber.pre-encryption.sqlite3.bak` so `mv` rolls back the change.
- `~/.kassiber/config/backends.env` and `~/.kassiber/attachments/` are
  **not** inside the SQLCipher boundary. They are outside the encrypted
  database file and remain plaintext on disk. URLs, kinds, chain, and
  network metadata are not secrets and may stay in the dotenv. Tokens,
  passwords, auth headers, and basic-auth usernames must move into the
  encrypted DB — use `kassiber secrets migrate-credentials` to lift any
  pre-existing entries in `backends.env` into the encrypted `backends`
  table, or seed new credentials directly with `--token-stdin` /
  `--token-fd FD`. Until that runs, every Kassiber command warns to
  stderr that the dotenv still carries plaintext secrets.
- A wrong passphrase produces the structured `unlock_failed` envelope
  rather than a partial open. The daemon refuses to start without a
  passphrase when the file is encrypted.
- `kassiber secrets change-passphrase` rotates the key in place via
  `PRAGMA rekey` and verifies with `cipher_integrity_check` when the
  bundled SQLCipher build supports it.
- A `.kassiber` backup file does **not** recover a forgotten passphrase.
  The DB inside the backup is encrypted under whatever passphrase was
  active when the backup was produced.

**OS keychain is not the perimeter.** This iteration deliberately does
not store unlock material in the OS keychain. The passphrase is the
perimeter. Pick a long passphrase from a password manager and treat
the loss of that passphrase as data loss — there is no recovery path.

**Reveal is a UX gate, not cryptographic separation.** Once the daemon
is running with the unlocked DB, it can read every credential. The
`auth_required` round-trip for `wallets reveal-descriptor` and
`backends reveal-token` enforces re-prompting for presence; it does not
add a separate cryptographic tier.

## Safe-to-record CLI output

Normal `backends ...` and `wallets ...` success output now follows a narrow
safe-to-record contract for secret-bearing config values:

- backend inspection output now uses an allowlisted safe view: raw credential
  values and unknown backend config keys are suppressed, while credential
  presence is exposed through `has_*` flags
- wallet inspection output now uses an allowlisted safe view: raw descriptor
  material and unknown wallet config keys are suppressed, while callers
  should rely on state flags such as `descriptor`, `change_descriptor`, and
  `descriptor_state` instead
- backend URLs shown in output drop embedded credentials and query strings

This contract is intentionally narrow. It does **not** mean every CLI surface
is safe to paste into a hosted model, issue tracker, or shared log. Addresses,
notes, file paths, backend names, and other operational metadata may still be
sensitive.

`kassiber diagnostics collect` is a separate public bug-report surface. Its
report is designed to be postable publicly: it includes version/platform data,
command shape, sanitized error context, stack module/function/line frames, DB
health, and aggregate state counts. It omits raw txids, addresses, descriptors,
xpubs, labels, notes, exact amounts, exact rates, backend hostnames, local
paths, raw config, raw API payloads, imported rows, and stack locals. `--save`
writes the artifact under `exports/diagnostics/` in the active Kassiber state
root. `--diagnostics-out auto` writes the same public report when a command
fails.

## Caveats

- **Secrets on the command line still end up in shell history if you
  use the deprecated argv forms.** `--token <value>`,
  `--auth-header <value>`, `--password <value>`, `--username <value>`,
  `--descriptor <value>`, and `--change-descriptor <value>` are kept
  for backwards-compatibility with existing scripts and emit a
  deprecation warning. Prefer the safe replacements:
  `--token-stdin` / `--token-fd FD` (and the matching `*-stdin` /
  `*-fd` variants for the other secret-bearing fields). Only one
  `--*-stdin` option may be active per invocation; any number of
  `--*-fd` options may coexist. The SQLCipher passphrase itself never
  has an argv form: use `--db-passphrase-fd FD` or the interactive
  prompt.
- **`--debug` is outside the safe-to-record contract.** Debug stack traces,
  exception context, and any future private logs may still include sensitive
  local state. Review before pasting into issues, screenshots, or logs. Use
  `diagnostics collect` or `--diagnostics-out auto` for public bug reports.
- **Normal machine output still carries sensitive operational metadata.**
  Success envelopes now redact secret-bearing backend and wallet config
  values, but addresses, paths, notes, and infrastructure choices can still
  be sensitive in hosted-model transcripts or shared logs.
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
- **Austrian tax processing is currently unavailable.** Kassiber only
  supports the generic RP2-backed path today. Future Austrian support is
  planned in the Kassiber-maintained RP2 fork at `bitcoinaustria/rp2`;
  until then, `tax_country=at` should be treated as unsupported.
- **Generic tax output is not tax advice.** It is accounting software
  output built on local wallet history and available pricing, not a
  substitute for jurisdiction-specific review.
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

## AI provider configuration

The desktop app and `kassiber ai` CLI surface speak the OpenAI-compatible
wire format against any provider you configure. The default seeded entry
points at local Ollama (`http://localhost:11434/v1`); add remote providers
through Settings → AI providers or `kassiber ai providers create`.

- **Prompts are sensitive accounting data.** A chat about quarantined
  transactions or report prep can include wallet labels, addresses, notes,
  imported document contents, backend hostnames, and tax annotations. Any
  remote provider sees that content. The provider/model picker tags each
  configured endpoint as `local`, `remote`, or `tee` so you can see at a
  glance whether a prompt is about to leave the device.
- **Remote chat only after explicit acknowledgement.** Remote providers
  start unacknowledged unless they are created or updated with
  `--acknowledge`, or confirmed in Settings → AI providers. `ai.chat`
  refuses to send prompts to an unacknowledged off-device provider with
  `ai_remote_ack_required`.
- **API keys live in plaintext SQLite** (mirroring how `backends` stores
  tokens) until the OS-keychain migration tracked in `TODO.md` covers
  both surfaces. Filesystem read of `~/.kassiber/data/kassiber.sqlite3`
  exposes any stored API key.
- **The Tauri shell allowlists exactly the AI daemon kinds.** The webview
  cannot reach Ollama (or any other model API) directly — every call
  passes through the Python daemon. The provider URL never reaches the
  webview's CSP/CORS surface.
- **Streaming Stop is UI-only, not a billing-side cancel.** Pressing Stop
  on the assistant marks the in-flight reply stopped and suppresses later
  streamed UI updates; the underlying generation keeps running until the
  model completes. For metered remote providers this means tokens continue
  to be consumed (and billed) after Stop. No prompt content is exposed beyond
  what was already sent. Cooperative cancellation lands with the worker-pool
  refactor in `TODO.md`.
- **No tool use in PR 1.** The in-app assistant cannot run Kassiber CLI
  commands, mutate state, or read your snapshots. That arrives in a
  follow-up gated behind per-tool consent.

## Reporting

Do not file security-impacting issues in the public tracker. Contact
the maintainer privately with a reproduction.
