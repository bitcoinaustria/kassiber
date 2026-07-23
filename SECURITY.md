# Security & Privacy

Kassiber is local-first. The database and all computation stay on your
machine. There is no telemetry, crash reporter, license check, or analytics.
Installed desktop and interactive CLI builds make a minimal GitHub release
check only after the app-wide permission is enabled; they do not download or install
updates. Outbound traffic is limited to the requests listed below.

Kassiber is pre-release (`0.1.x`) — treat this as a description of
current behavior, not a long-term contract.

## The big gotcha: not running your own node

Out of the box, Kassiber ships four built-in named backends:

- `mempool` → `esplora` → `https://mempool.bitcoin-austria.at/api` — the
  default for Bitcoin wallets, operated by Bitcoin Austria.
- `fulcrum` → `electrum` → `ssl://index.bitcoin-austria.at:50002` —
  Bitcoin-Austria-operated Electrum/Fulcrum indexer.
- `liquid` → `electrum` → `ssl://les.bullbitcoin.com:995` — a
  third-party Liquid Electrum endpoint operated by BullBitcoin.
- `liquid-blockstream` → `electrum` → `ssl://blockstream.info:995` —
  a third-party Liquid Electrum endpoint operated by Blockstream.

Every `wallets sync` against any of these sends your addresses (or
scripthashes, or gap-limit-scanned descriptor scripts) to whoever
operates that endpoint. They can link the queries to your IP and the
identifying `User-Agent: kassiber/<version>` header. "Bitcoin Austria
operates it" is still third-party from *your* machine's point of view;
"BullBitcoin operates it" and "Blockstream operates it" for Liquid
especially so.

Descriptor wallets are worse than address wallets here: gap-limit
discovery leaks a contiguous run of receive + change scripts, so the
backend sees the wallet cluster rather than just individual addresses.
Silent Payments wallets are different again: ordinary scripthash backends
cannot discover BIP352 outputs. Kassiber requires an explicitly configured
Silent-Payments-capable backend or local scanner and will not silently fall
back to the built-in clearnet defaults.
Server-assisted Silent Payments scans also trust the selected backend for
completeness: if it omits scan candidates, Kassiber cannot independently prove
that a reported-complete range found every payment. Use a local scanner or a
self-hosted SP indexer for accounting-critical books.

Mitigations, in order of effect:

1. Run your own Bitcoin Core and use a `bitcoinrpc` backend (traffic
   stays on-box).
2. Run your own Esplora / Electrs / Fulcrum and use it as an `esplora`
   or `electrum` backend.
3. Deliberately set `--tor-proxy` / `tor_proxy` on each supported backend you
   want routed, including `.onion` Esplora/Electrum/Fulcrum backends, or torify
   the whole process (`torsocks python3 -m kassiber ...`) / route through a VPN.
4. Prefer `address`-kind wallets over `descriptor`-kind wallets when
   you only care about a fixed set of addresses.
5. Skip `rates sync` and use `rates set` for manual rate upserts.

## External requests (complete list)

All HTTP(S) requests send `User-Agent: kassiber/<version>`. This is not
configurable.

| Trigger | Destination | Transport | What the other side learns |
| --- | --- | --- | --- |
| Desktop launch after 10 seconds and every 24 hours while open when the setup / **Settings → Privacy → Allow GitHub update checks** permission is enabled; macOS **Check for Updates…** checks only under the same permission | stable builds: `https://api.github.com/repos/bitcoinaustria/kassiber/releases/latest`; prerelease builds: `https://api.github.com/repos/bitcoinaustria/kassiber/releases?per_page=10` (GitHub) | unauthenticated HTTPS GET; redirects refused | IP, User-Agent, request timing, and that a Kassiber release check occurred; no project, wallet, book, build hash, hostname, device, or installation identifier is sent |
| Packaged CLI in human-readable table mode on a TTY when the same permission is enabled and its public release cache is absent or older than 20 hours; failed attempts back off for one hour; `kassiber update` checks only while enabled | matching stable/prerelease GitHub endpoint above | detached unauthenticated HTTPS GET with redirects refused for automatic checks; foreground GET for the explicit command | same release-check metadata as above; machine, structured-format, non-interactive, daemon, operator-child, redirected-output, and source-checkout runs do not check automatically |
| `wallets sync` against the built-in `mempool` default | `https://mempool.bitcoin-austria.at/api` (Bitcoin Austria) | Esplora over HTTPS | IP, User-Agent, scripthashes, query timing, descriptor scan shape |
| `wallets sync` against the built-in `fulcrum` default | `ssl://index.bitcoin-austria.at:50002` (Bitcoin Austria) | Electrum JSON-RPC over TLS | IP, queried scripthashes, query timing |
| `wallets sync` against the built-in `liquid` default | `ssl://les.bullbitcoin.com:995` (BullBitcoin) | Electrum JSON-RPC over TLS | IP, queried Liquid scripthashes, query timing |
| `wallets sync` against the built-in `liquid-blockstream` default | `ssl://blockstream.info:995` (Blockstream) | Electrum JSON-RPC over TLS | IP, queried Liquid scripthashes, query timing |
| `wallets sync` against a user-configured Esplora backend | your configured URL | Esplora over HTTP(S) | same categories as `mempool` above |
| `wallets sync` against a user-configured Electrum backend | your configured `ssl://` or `tcp://` URL | Electrum JSON-RPC over raw TCP/TLS | IP, queried scripthashes, query timing |
| `wallets sync` against a `bitcoinrpc` backend | your configured URL | HTTP(S) POST with Basic auth | nothing leaves your machine if the node is local |
| `wallets sync` for a `silent-payment` wallet with a local scanner file | local filesystem path configured by you | local file read | no network request by Kassiber; scanner output and detected Taproot outputs stay local; on POSIX the scanner file must be user-owned and `0600` |
| `wallets sync` for a `silent-payment` wallet in server-assisted mode | your explicitly configured HTTP(S) SP-capable backend URL/path | HTTP(S) POST through that backend's proxy setting, if any | IP, User-Agent, scan request timing, scan birthday/range, the watch-only `sp()` scan material needed by that backend, and scan completeness depends on that backend not omitting candidates |
| `rates sync` (only) | configured provider (`mempool` backend, Coinbase Exchange, or CoinGecko) | unauthenticated HTTP(S) GET | IP, User-Agent, which fiat pair and window |
| `ai models`, `chat`, `ai.test_connection` against a configured remote/TEE provider | your configured provider URL or CLI provider | OpenAI Responses-compatible HTTP(S) or the configured local CLI's own transport | prompt/tool context, model request metadata, IP/provider account context according to that provider |
| consented mutating AI tools inside `chat` or the desktop Assistant (`ui.wallets.sync`, `ui.rates.rebuild`, `ui.maintenance.run`) | the backends/rate sources of the rows above | as in those rows | as in those rows — tool consent is also network consent for that row |

The app-wide consent at `~/.kassiber/config/update-checks.json` is owner-only
and contains only a schema version and boolean. Missing, malformed, symlinked,
or explicitly disabled consent fails closed in both the native desktop command
and packaged CLI. The renderer reads this file through the native boundary and
never restores consent from browser storage. Setup persists the choice before
creating or mutating book state. All GitHub release requests go through the
CLI's single code path — the desktop invokes its bundled CLI sidecar rather
than carrying a second HTTP client. A sibling owner-only `update-checks.lock`
serializes checks with preference writes: disabling waits for an
already-authorized request to finish, and after it returns no later request can
start until consent is enabled again. Disabling blocks automatic checks, the
macOS menu action, plain `kassiber update`, and detached CLI refresh workers.
CLI-only users manage it locally with `kassiber update --enable-checks`,
`--disable-checks`, or `--status`; the latter two never contact GitHub.

The CLI cache at `~/.kassiber/config/update-check.json` contains only the public
release version, URL, prerelease flag, and check time; it is written mode `0600`
where supported. A sibling owner-only `.attempt` file contains only the last
automatic-attempt time so an unavailable GitHub endpoint cannot turn every CLI
invocation into another request. `KASSIBER_DISABLE_UPDATE_CHECK=1` is an
additional process-level override that suppresses automatic and explicit checks
even when persisted consent is enabled.

The update announcement itself is not cryptographically signed. The notifier
therefore trusts HTTPS and control of the Kassiber GitHub repository only to
decide whether to show a release link. It never treats the response as
permission to download or execute anything.

The release workflow generates a Sparrow-style versioned SHA-256 manifest, and
`kassiber verify-download` can authenticate a detached OpenPGP signature before
checking an artifact hash. No permanent Kassiber release public key or
fingerprint has been published yet, so current manifests and packages remain
unauthenticated. During this transition the verifier requires both a local
public-key file and the full primary-key fingerprint obtained independently. It
inspects and dearmors the key into a temporary isolated keyring, performs no
network lookup, pins the full fingerprint, verifies the manifest with `gpgv`,
and only then hashes the selected artifact. See
[release signing](docs/reference/release-signing.md).

Release finalization and external Linux channel publication use the same
code-reviewed public key and primary fingerprint once that policy is enabled.
The general release private key never enters CI. A separate protected archive
key signs mutable APT/DNF metadata and RPMs; it has a distinct primary identity
so compromise of that CI-held subkey cannot authenticate a general release
manifest.

On Linux, the update checker treats `.deb`/`.rpm` installs as manual and shows
the GitHub release link with no `apt`/`dnf` command. Package contents cannot
prove repository provenance, so no package-manager command is offered until a
live signed origin and archive-key fingerprint are pinned and verified in
code.

Both update clients honor the process's standard system proxy environment. A
configured proxy can therefore observe the GitHub destination and request
timing and may receive proxy credentials from its own configuration. Per-backend
`tor_proxy` settings do not route the global update checker. Opening the release
link is explicit and hands control to the default browser, including that
browser's normal GitHub cookies and privacy context.

No other Kassiber-owned path makes network calls. `rates set`, `rates latest`,
`rates range`, `rates pairs`, journal processing, metadata CRUD, and all
reports are fully offline unless the user explicitly invokes an AI provider
that itself contacts a remote service.

Backend `tor_proxy` values are a deliberate per-backend routing choice. They
are honored by Electrum sockets, Esplora / Explorer-API HTTP reads (Bitcoin and
Liquid), BTCPay Greenfield HTTP sync, Bitcoin Core RPC HTTP calls, and
mempool-rate fetches that use a configured mempool backend. Partial routing is
supported: configuring a proxy on one backend does not route any other backend,
AI provider, or standalone rate provider. Proxy values may be `HOST:PORT`,
`socks5://...`, `socks5h://...`, `socks5h://USER:PASS@HOST:PORT`, or
`http(s)://...`; encode special characters in usernames/passwords. Standalone
Coinbase/CoinGecko providers do not yet have a per-provider proxy setting. The
desktop setup forms detect `.onion` backend hosts and prefill the standard local
Tor SOCKS proxy (`127.0.0.1:9050`) for that backend only; Kassiber does not
start or bundle Tor, so the user still needs an existing Tor service.

## Local storage

- `~/.kassiber/config/projects.json` — global project catalog. Contains only
  project id/name/path/encrypted status/last-opened metadata. It must never
  contain passphrases, verifier hashes, wrapped keys, descriptors, xpubs,
  backend tokens, accounting rows, or chat content. Project ids/names and
  last-opened timestamps are still local metadata: choose labels accordingly.
  Kassiber writes the catalog with best-effort owner-only permissions.
- `~/.kassiber/projects/<project>/data/kassiber.sqlite3` — project SQLite DB.
  Contains the books/profiles in that project: descriptors, xpubs, addresses,
  transactions, metadata, rates cache, backend definitions/defaults, and any
  stored backend credentials. Versioned BDK/LWK observer state and opaque LWK
  values live only in this database; the bindings receive no path and may not
  create a side database, wallet file, cache, or state directory. These derived
  rows are excluded from diagnostics, audit packages, AI/desktop payloads, and
  cross-device authored-event replication.
- Persisted AI chat sessions also live inside that database
  (`ai_chat_sessions` / `ai_chat_messages`) — never as separate plaintext
  files. The default `auto` policy persists only when the database is
  SQLCipher-encrypted; `kassiber chats config --history off` disables it,
  `kassiber chat --incognito` skips one session, and `kassiber chats
  delete/clear` remove stored sessions. Diagnostics reports and audit
  packages do not include chat content. The opt-in `kassiber chat
  --transcript <path>` file is the one plaintext chat artifact, written only
  where the user points it.
- `~/.kassiber/projects/<project>/config/backends.env` — project-local backend
  bootstrap file. It is plaintext. It may contain Bitcoin Core RPC credentials
  and backend tokens until `kassiber secrets migrate-credentials` lifts them
  into the encrypted project DB.
- `~/.kassiber/projects/<project>/config/settings.json` — managed state
  manifest for the project path layout. Not secret by itself, but it reveals
  where the rest of that project lives.
- `~/.kassiber/projects/<project>/attachments/` — managed attachment store for
  copied local files. URL attachments are stored as literal references in the
  database and are not fetched. Attachment files are plaintext unless the user
  protects the project directory with OS or volume encryption.
- `~/.kassiber/projects/<project>/exports/` — generated reports and handoff
  artifacts. These are plaintext user outputs and are outside SQLCipher.
- Liquid descriptor wallets embed **private SLIP77 blinding keys** in
  `wallets.config_json`. Anyone who can read the DB can unblind your
  confidential outputs.
- Silent Payments wallets store BIP392 watch-only scan material such as
  `sp(...)` / `spscan` in `wallets.config_json`. It cannot spend, but it is
  privacy-sensitive because it can reveal matching receives to anyone who scans
  the chain with it.
- Silent Payments local scanner JSON files are not stored inside Kassiber's
  database. If you configure `silent_payment_scan_file`, protect that file with
  OS permissions and keep it out of shared or cloud-synced folders. On POSIX,
  Kassiber refuses scanner files that are not regular files owned by the current
  user or that grant any group/other permissions.
- Older installs may still resolve to `~/.local/share/kassiber`,
  `~/.local/share/satbooks`, or a legacy `<data-root>/.env`; run
  `kassiber status` to see the active paths.
- Keep backend config out of version control. Prefer `COOKIEFILE` over inline
  `USERNAME` / `PASSWORD`.

### At-rest encryption — passphrase-gated SQLCipher (V4.1)

Each project database is optionally encrypted via SQLCipher 4, and each
encrypted project has its own passphrase. After `kassiber secrets init`, select
an explicit unlock mode for that project: `manual` prompts or uses
`--db-passphrase-fd` per process, `brokered` uses a capability-scoped in-memory
operator lease, and `unattended` opts into the separate CLI remembered-unlock
credential. Brokered mode never falls through to remembered unlock. Unlocking
one project does not unlock another.

- `~/.kassiber/projects/<project>/data/kassiber.sqlite3` — when encrypted, contents are
  protected by SQLCipher 4 with stock PBKDF2-HMAC-SHA512
  (`kdf_iter = 256000`). Recoverable with the upstream `sqlcipher`
  binary using only the passphrase.
- The pre-migration plaintext file is preserved as
  `kassiber.pre-encryption.sqlite3.bak` so `mv` rolls back the change.
  Kassiber refuses to overwrite an existing rollback backup at that path.
- Legacy app-wide project migration moves old active plaintext database/config
  artifacts aside under `pre-project-migration-<timestamp>/` after the new
  project copy is staged, instead of leaving the old active paths silently live.
- `projects.json`, `config/backends.env`, `config/settings.json`,
  `attachments/`, and `exports/` are **not** inside the SQLCipher boundary.
  They are outside the encrypted database file and remain plaintext on disk.
  URLs, kinds, chain, and network metadata are not secrets and may stay in the
  dotenv. Tokens, passwords, auth headers, and basic-auth usernames must move
  into the encrypted DB — use `kassiber secrets migrate-credentials` to lift
  any pre-existing entries in `backends.env` into the encrypted `backends`
  table, or seed new credentials directly with `--token-stdin` / `--token-fd
  FD`. Until that runs, every Kassiber command warns to stderr that the dotenv
  still carries plaintext secrets.
- A wrong passphrase produces the structured `unlock_failed` envelope
  rather than a partial open. The daemon refuses to start without a
  passphrase when the file is encrypted.
- `kassiber secrets change-passphrase` rotates the key in place via
  `PRAGMA rekey` and verifies with `cipher_integrity_check` when the
  bundled SQLCipher build supports it.
- A `.kassiber` backup file does **not** recover a forgotten passphrase.
  The DB inside the backup is encrypted under whatever project passphrase was
  active when the backup was produced.

Switching projects in the desktop daemon closes the current SQLite connection,
stops background freshness workers, clears the in-memory passphrase, and then
requires the selected project's passphrase before reads, AI tools, reports,
exports, or backups can touch that project.

**OS keychain is not the perimeter.** The SQLCipher passphrase is the
perimeter. Pick a long passphrase from a password manager and treat
the loss of that passphrase as data loss — there is no recovery path.
Desktop macOS builds can optionally remember the database passphrase in a
desktop-only Keychain item for Touch ID unlock. Production-entitled builds use
an item-level `biometryCurrentSet` policy, which invalidates access when enrolled
fingerprints change. Unsigned/ad-hoc preview builds cannot use the protected
Keychain and retain an explicit app-level LocalAuthentication check before
reading their desktop-only item. The CLI separately opts into its own
per-data-root item on macOS, Windows user-scope Credential Manager, or an
available unlocked Linux Secret Service. The CLI opt-in
is a non-secret boolean in managed `config/settings.json`; desktop-only Touch ID
enrollment leaves it unset. Only the native `keyring` backend for the current
platform is accepted (including a chainer only when every active child is
native); environment/config-selected third-party and file backends are rejected.
There is never a plaintext-file fallback.

`kassiber secrets status` exposes the active CLI boundary as a stable
`access_policy` code: `macos_keychain_application_acl`,
`windows_dpapi_user_scope`, `linux_secret_service_session`, or `unsupported`.
This is public-safe capability metadata, not evidence that the item is
biometric-gated.

CLI credential reads are **not biometric-gated**. On macOS the CLI item uses the
Keychain's per-binary access policy, and unsigned/ad-hoc preview binaries may
prompt again after rebuilds or identity changes. On Windows and Linux, another
process running as the same user can read the user-scoped item. This matches the
existing boundary above: a compromised process running as the user is out of
scope. Revoke only the CLI copy with `kassiber secrets forget-unlock`, revoke
only the desktop copy in Settings, or use **Forget all unlock methods** to remove
both plus the migration-only legacy item. The current credential names are
`Kassiber CLI Database Passphrase` and `Kassiber Desktop Biometric Passphrase`;
`Kassiber Database Passphrase` is legacy migration input only. Revocation does not
change the SQLCipher key or recover a lost passphrase.
If verified deletion of a CLI-owned legacy item fails, Kassiber atomically
disables CLI remembered unlock and sets the non-secret
`cli_legacy_unlock_quarantined` ownership marker. The CLI will not read or
migrate that retained value, and the desktop will not claim it. Status exposes
the quarantine so the user can remove the item manually and retry cleanup.

Desktop passphrase rotation refreshes both enrolled namespaces. A CLI rotation
cannot rewrite a biometric-protected desktop item without defeating that access
policy, so both CLI and desktop rotation arm a non-secret
`desktop_biometric_stale` guard in managed settings before SQLCipher is rekeyed.
Its value is an opaque generation token, so an older enrollment callback cannot
clear a newer rotation's guard. Because a post-rekey verification failure is
ambiguous, the guard stays armed on any rotation error and is compare-and-cleared
only after the Tauri process successfully refreshes its credential, or cleared
after verified removal. The desktop therefore requires manual passphrase entry
and re-enrollment rather than attempting a known-stale biometric copy, including
after a process crash between rekey and Keychain refresh. A preview build cannot
replace an existing protected enrollment, and credential removal does not clear
enrollment markers until the applicable fallback and protected copies have been
cleaned up successfully.
Managed-settings reads for this guard fail closed, and Tauri keeps the lexical
data-root for the settings path even when the Keychain account uses a canonical
path; a symlinked final `data` directory therefore cannot split the two sides of
the guard channel.

**Desktop credential stores are a separate boundary, not SQLCipher
replacement.** Desktop builds can store AI provider API keys in macOS
Keychain, Windows user-scope Credential Manager/DPAPI, or Linux Secret Service
when platform policy selects a native store. The unlocked Python daemon remains
trusted at runtime and receives the key only to call the configured provider.
Backend tokens, descriptors, xpubs, blinding keys, and reveal payloads stay
SQLCipher-protected and are not migrated to OS credential stores. See
[docs/plan/10-secret-management.md](docs/plan/10-secret-management.md).

**Reveal is a UX gate, not cryptographic separation.** Once the daemon
is running with the unlocked DB, it can read every credential. The
`auth_required` round-trip for `wallets reveal-descriptor` and
`backends reveal-token` enforces re-prompting for presence; it does not
add a separate cryptographic tier.

### Terminal operator broker

Brokered unlock keeps a project's passphrase only in a per-user broker process
for an explicit duration or until-lock session. It does not authenticate an
individual agent: any process intentionally running as the same logged-in OS
user can exercise the active lease's capabilities. Cross-user isolation comes
from owner-only local IPC, peer user-id/SID validation, separate native
credential namespaces, and per-project ownership locks.
Those identity/path locks are acquired before database open and inherited by
worker children, so broker death cannot release a project to a second owner
while an orphan mutation still runs. On macOS the broker spawns the signed
Touch ID helper with a broker-created inherited output pipe. The signed CLI and
broker bind to the live launcher's
verified bundle identifier, TeamIdentifier, and code-directory hash. The broker
dynamically validates the spawned helper PID against that Developer ID
requirement before writing an enrollment passphrase, closing the mutable-path
check/use gap. An inherited readiness/release gate blocks every Keychain action
until the live helper check completes. The helper accepts the request only when its parent is the
matching production-signed bundled CLI
sidecar: it validates the bundle path and signing team, then uses
Security.framework to check the live parent process against a fixed Developer
ID Application requirement for the exact sidecar signing identifier and the
helper's verified TeamIdentifier. It exposes no caller-selected endpoint or general
raw-secret return action.

Brokered mode never reads the unattended CLI remembered-unlock item. Manual,
brokered, and unattended modes are distinct and visible in `kassiber operator
status`. Normal leases may grant read, operator, and accounting-decision work;
admin operations require a fresh one-operation authorization that expires if
the operation waits in the queue for more than 60 seconds. Broker death,
logout, reboot, explicit lock, or lease expiry removes the in-memory grant.
There is no exactly-once claim across a broker crash; unproven nonzero exits
from mutating children are reported as `result_unknown`. See
[docs/reference/operator-broker.md](docs/reference/operator-broker.md) for the
protocol, platform primitives, queue semantics, and memory-zeroization limits.
On Linux the broker watches logind's per-user login state and the original
device/inode identity of the XDG runtime root, so logout or runtime-directory
replacement tears down leases even with a lingering user manager. Broker
startup fails closed when neither mechanism can prove logout lifetime; manual
mode remains available on such unusual no-PAM systems.

## Safe-to-record CLI output

Normal `backends ...` and `wallets ...` success output now follows a narrow
safe-to-record contract for secret-bearing config values:

- backend inspection output now uses an allowlisted safe view: raw credential
  values and unknown backend config keys are suppressed, while credential
  presence is exposed through `has_*` flags
- wallet inspection output now uses an allowlisted safe view: raw descriptor
  material, Silent Payments scan material, and unknown wallet config keys are
  suppressed, while callers should rely on state flags such as `descriptor`,
  `change_descriptor`, `descriptor_state`, and `silent_payment` instead
- backend URLs shown in output drop embedded credentials and query strings

This contract is intentionally narrow. It does **not** mean every CLI surface
is safe to paste into a hosted model, issue tracker, or shared log. Addresses,
notes, file paths, backend names, and other operational metadata may still be
sensitive.

`kassiber diagnostics collect` is a separate public bug-report surface. Its
report is designed to be postable publicly: it includes version/platform data,
command shape, sanitized error context, stack module/function/line frames, DB
health, and aggregate state counts. It omits raw txids, addresses, descriptors,
xpubs, Silent Payments scan material, labels, notes, exact amounts, exact
rates, backend hostnames, local paths, raw config, raw API payloads, imported
rows, and stack locals. `--save` writes the artifact under
`exports/diagnostics/` in the active Kassiber state root.
`--diagnostics-out auto` writes the same public report when a command fails.

## Caveats

- **Secrets on the command line still end up in shell history if you
  use argv forms.** `--token <value>`,
  `--auth-header <value>`, `--password <value>`, `--username <value>`,
  `--descriptor <value>`, `--change-descriptor <value>`,
  `--sp-descriptor <value>`, and `--api-key <value>` should be avoided for
  real secrets. Prefer the safe replacements:
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
- **Proxy scope is per backend.** `backends create --tor-proxy` protects
  supported backend-backed transports, but it is not a bundled Tor daemon and
  it is not a global proxy for every future network integration. Use an
  existing Tor/SOCKS service you control, or torify the whole process for
  surfaces without a per-provider proxy setting.
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
- **Lightning node wallet kinds.** `coreln` can sync through read-only
  Core Lightning RPC methods. Prefer a commando rune restricted to list,
  get, and `bkpr-list*` methods with a rate cap (e.g.
  `restrictions='[["method^list","method^get","method^bkpr-list","method=summary"],["method/listdatastore"],["rate=60"]]'`).
  Kassiber passes the rune through the `LIGHTNING_RUNE` environment
  variable so it does not appear in `/proc/<pid>/cmdline`. Local
  `lightning-rpc` file access is also supported but is not least-privilege
  on its own. `lnd` and `nwc` remain declared but inactive.

## AI provider configuration

The desktop app and `kassiber ai` CLI surface use `POST /v1/responses` against
HTTP providers. The default seeded entry points at local Ollama
(`http://localhost:11434/v1`); add remote providers through Settings → AI
providers or `kassiber ai providers create`. Responses are stored by default
by OpenAI, so Kassiber explicitly sends `store: false`. It does not use
provider-side conversation IDs; typed reasoning and function-call Items needed
for a tool round-trip are replayed only in daemon memory.

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
- **AI provider API keys have a narrow desktop native-store path.** CLI
  callers should use `--api-key-stdin` / `--api-key-fd FD`; the old
  `--api-key <value>` form is a warning-on-use shim and stores
  `sqlcipher_inline`. Desktop Settings writes keys through
  `ai.providers.set_api_key` and can move a provider key with
  `ai.providers.move_api_key`; provider envelopes return only `has_api_key`
  plus `secret_ref.{store_id,state}`. No generic keyring get/list API is
  exposed to the webview or assistant.
- **No encrypted-while-running claim.** Once the Python daemon is unlocked, it
  can read stored AI keys, backend tokens, descriptors, and blinding keys to do
  its job. This does not protect against malware, admin/root access, debugger
  memory inspection, a compromised OS, or a compromised webview process.
- **The Tauri shell allowlists exactly the AI daemon kinds.** The webview
  cannot reach Ollama (or any other model API) directly — every call
  passes through the Python daemon. The provider URL never reaches the
  webview's CSP/CORS surface. The in-app AI has no shell, raw filesystem,
  arbitrary CLI, or generic daemon-dispatch access.
- **The Vite daemon bridge is development-only.** `pnpm --dir ui-tauri run
  dev:bridge` exposes selected daemon kinds, including AI streaming and
  consent controls, through the Vite server on loopback for browser testing.
  Do not bind that server to a LAN address or use it as a REST API; it is only
  a local development bridge to the same daemon trust boundary.
- **Streaming Stop is best-effort cooperative cancel, not a billing
  guarantee.** Pressing Stop sends `ai.chat.cancel` to the local daemon and
  suppresses later streamed UI updates. The Python worker stops forwarding
  deltas once provider control returns between chunks and marks the terminal
  response `finish_reason: "cancelled"`. Remote providers may still bill for
  tokens already generated or in flight. No prompt content is exposed beyond
  what was already sent.
- **Read-only AI tools send selected local data to the selected provider.**
  When tools are enabled, the assistant may read safe daemon snapshots such as
  status, overview, filtered transactions, wallet/backend summaries, profiles,
  journals, quarantine summaries, transfer-pair summaries, cached rate metadata,
  workspace health, next-action guidance, capital-gains reports, and allowlisted
  skill references. If the selected provider is remote or TEE, those tool
  results are sent to that provider as chat context.
- **AI read tools use redacted daemon surfaces.** They must not expose secrets,
  descriptors, xpub material, API keys, tokens, cookies, auth headers, exact
  backend URLs, wallet config JSON, or raw wallet files. Wallet and backend
  tools return labels, kinds, URL presence flags, credential presence flags,
  and status-style metadata only.
- **Mutating AI tools require explicit consent.** The current mutating surface
  is limited to `ui.wallets.sync`. Each call emits a redacted preview and waits
  for `allow_once`, `allow_session`, or `deny`; session consent lasts only for
  that one chat request and only for the same tool name. If allowed, the tool
  result is fed back to the selected provider as chat context. Unknown tools
  still return `tool_not_allowed` and never execute.

## Reporting

Do not file security-impacting issues in the public tracker. Contact
the maintainer privately with a reproduction.
