# Rust backend contract inventory

> **Historical.** This page records the Python-to-Rust port. Python was removed
> under [plan 22](../plan/22-python-removal.md); the `kassiber/` paths and Python
> tests it cites exist only at the `archive/python-backend` tag.

R0 reference at base `8709d430`. This records observed Python behavior, including
inconsistencies; it does not authorize changes to privacy or accounting policy.
Line references are to that base's product code. See [plan 21](../plan/21-rust-backend.md),
[test dispositions](rust-port.md), [machine output](machine-output.md),
[daemon](daemon.md), and [operator broker](operator-broker.md).

## Generated evidence

`uv run --locked python -m tests.rust_port.contracts update` regenerates all four
files under `tests/fixtures/rust_port/`; `check` compares without writing.
`tests/rust_port/test_contracts.py` runs that comparison in preflight. There is
no book open or network access: catalog capture uses a disposable home and an
audit hook that refuses every socket event, including loopback.

| Fixture | Scope and interpretation |
| --- | --- |
| `commands_describe.json` | Exact stdout bytes of `uv run --locked python -m kassiber --machine commands describe`: 313 leaves, schema 1, English help and argument metadata. No timestamp or environment masking. |
| `daemon_kinds.json` | All 303 supported request kinds, each terminal kind, all Tauri/Vite invocation, streaming and AI gates, and static emission sites classified as terminal, startup, event, stream/control or dynamic. Dynamic expressions are retained for inspection. |
| `error_codes.json` | Code keyword arguments, local positional wrappers, error defaults and dictionary code fields, with source locations and retry policy. Includes issue/warning codes to avoid silently losing them. `false` is the AppError/envelope default, `null` means a record has no retryable field; an expression is a runtime policy, never a guessed boolean. |
| `environment.json` | Actual environment access sites/default expressions and a broader literal occurrence index (including writes/comments), plus the frozen Python launcher. Provider/backend variable families and implicit standard-library inputs are explained below. A literal occurrence alone does not establish a supported knob. |

These fixtures inventory declarations and construction sites; they do not prove
that every handler executes correctly. Session replay, historical DB tests,
tax oracles and the native egress guard remain separate R0 deliverables.

## Environment and precedence

The backend directly accesses the variables/families below. “User” means the
launching process's environment, not a value stored in a book. Empty overrides
usually fall through; exceptions and truth sets matter. No env var transports
a database passphrase.

| Variable | Precedence, default and setter; evidence |
| --- | --- |
| `HOME` | POSIX `Path.home()`/`expanduser()` uses it, then the account database; affects default state and `~` paths. Set by OS/user/tests. Ownership locks deliberately use the account database instead. `kassiber/db.py:63`, `kassiber/operator/project.py:574`. |
| `USERPROFILE`, `HOMEDRIVE`, `HOMEPATH` | Windows Python home/`expanduser()` inputs: USERPROFILE, then HOMEDRIVE+HOMEPATH. OS/user/tests set these. They are implicit Python library inputs, not explicit Kassiber getenv calls; `kassiber/db.py:68`. Rust must retain the resulting home/path behavior. |
| `XDG_DATA_HOME` | Linux absolute value wins; missing/relative falls back to `~/.local/share`, then `/kassiber`. User/OS/tests. No `XDG_CONFIG_HOME` lookup for backend preferences. `kassiber/db.py:76`. |
| `LOCALAPPDATA` | Windows absolute value wins; missing/relative falls back to `~/AppData/Local`, then `/at.bitcoinaustria.kassiber`. OS/tests. Owner locks use the shell API instead. `kassiber/db.py:72`, `kassiber/operator/project.py:621`. |
| `XDG_RUNTIME_DIR` | OS login-session directory, not the broker socket root. Linux broker validates `/run/user/<uid>` and pinned device/inode as a logout fallback; missing/unsafe needs logind or startup fails. AI broker uses an absolute real directory owned by the user, not writable by others, on tmpfs/ramfs; otherwise system temp. `kassiber/operator/server.py:726`, `kassiber/ai/broker_client.py:138`. |
| `KASSIBER_OFFLINE_PREFERENCE_FILE` | Nonempty expanded path overrides global `config/offline-mode.json`; users/tests can isolate it. `kassiber/egress_policy.py:40`. |
| `KASSIBER_NO_EGRESS` | Trimmed, case-insensitive `1,true,yes,on` blocks shared backend transports including loopback, before offline-file evaluation. User/tests; broker can tighten a child to `1`. This override does not cover every provider/update path; offline mode does, subject to the local-provider exception. `kassiber/egress_policy.py:84`, `kassiber/egress_policy.py:154`, `kassiber/operator/runner.py:64`. |
| `KASSIBER_UPDATE_PREFERENCE_FILE` | Nonempty expanded path overrides `config/update-checks.json` (consent). Users/tests. `kassiber/update_check.py:174`. |
| `KASSIBER_UPDATE_CACHE_FILE` | Nonempty expanded path overrides `config/update-check.json` (cached result). Users/tests. `kassiber/update_check.py:167`. |
| `KASSIBER_DISABLE_UPDATE_CHECK` | Trimmed/lower `1,true,yes` overrides consent to false; `on` is not in this truth set. Users/tests/packaging. `kassiber/update_check.py:181`. |
| `KASSIBER_HOMEBREW_PACKAGE` | `cask`/`formula` overrides install-method inference; otherwise inspect executable/argv paths for Cellar formula; default `manual`. Packaged cask launcher sets `cask`. `kassiber/update_check.py:468`, `ui-tauri/src-tauri/bin/kassiber:19`. |
| `KASSIBER_AGENT_ACCESS_FILE` | Nonempty expanded path overrides global `config/agent-access.json`; users/tests. `kassiber/agent_access.py:30`. |
| `SATBOOKS_BACKEND_<NAME>_<FIELD>`, `KASSIBER_BACKEND_<NAME>_<FIELD>` | Dynamic families; split suffix on its first underscore, lowercase backend name, canonicalize field aliases (`RPCPASSWORD` → `password`, etc.). Within each layer Kassiber prefix overrides legacy. Layers: built-ins, selected dotenv, persisted DB, process overrides. Process values remain ephemeral; they must not become saved credentials/config through bootstrap promotion. Users/config tools/tests. `kassiber/backends.py:248`, `kassiber/backends.py:299`, `kassiber/backends.py:1184`. |
| `SATBOOKS_DEFAULT_BACKEND`, `KASSIBER_DEFAULT_BACKEND` | Current nonempty value precedes legacy nonempty value, in process before dotenv/bootstrap; fallback built-in `fulcrum`. Process presence also marks the override even if its value is blank. Saved DB default participates when merging unless process override controls it; manual mode/tombstones prevent reseeding public presets. `kassiber/backends.py:325`, `kassiber/backends.py:1184`. |
| `KASSIBER_DEFAULT_AI_PROVIDER` | Trim/lower, defaults `ollama`, must name a bootstrap provider or validation fails. Users/tests set bootstrap choices; existing provider records are not blanket overwritten. `kassiber/ai/providers.py:86`, `kassiber/ai/providers.py:179`. |
| `KASSIBER_<PROVIDER>_AI_BASE_URL`, `KASSIBER_DEFAULT_AI_BASE_URL` | Specific nonempty value first; default URL override only for the selected default bootstrap provider; otherwise provider literal. Provider token is uppercase with `-` → `_` (e.g. OLLAMA, OMLX). Configuration never authorizes contact. Users/tests. `kassiber/ai/providers.py:192`. |
| `KASSIBER_AI_BROKER_NODE` | Expanded executable file if usable, otherwise `node` on PATH, then fixed fallback locations. Users/packaging/tests. `kassiber/ai/broker_client.py:89`. Rust: removed with the Node broker ([plan 23](../plan/23-rust-provider-broker.md)). |
| `KASSIBER_AI_PROVIDER_BROKER` | Expanded script override, otherwise sibling `provider_broker/index.mjs` (Rust since [plan 23](../plan/23-rust-provider-broker.md): an executable run without arguments, otherwise this executable with `--provider-broker`). Users/packaging/tests. `kassiber/ai/broker_client.py:105`, `crates/net/src/broker.rs`. |
| `KASSIBER_AI_BROKER_TMPDIR` | Written by Python to its TypeScript provider-broker child's environment, not read by Python; private per-turn working root, removed after use. `kassiber/ai/broker_client.py:185`. Rust: the same, for the `kassiber --provider-broker` child. |
| `KASSIBER_OPERATOR_RUNTIME_DIR`, `KASSIBER_OPERATOR_ALLOW_TEST_RUNTIME_DIR` | Path honored only in unfrozen source builds with the gate exactly `1`; otherwise Unix socket root is account-home `~/.kassiber/run`. Tests set both. Never retargets ownership locks. Windows pipe uses user SID. `kassiber/operator/protocol.py:218`. |
| `KASSIBER_OPERATOR_DIRECT` | Exactly `1` bypasses broker routing. Broker sets it for worker children; default normal mode routing. This is not a replacement for child identity checks. `kassiber/operator/cli.py:188`, `kassiber/operator/runner.py:56`. |
| `KASSIBER_OPERATOR_CHILD` | Exactly `1` selects broker-child identity preflight, raw Windows secret-handle conversion, and suppresses ordinary update banners. Broker sets it; absent is ordinary CLI. `kassiber/cli/main.py:6695`, `kassiber/secrets/prompt.py:61`, `kassiber/update_check.py:552`. |
| `KASSIBER_OPERATOR_EXPECTED_PROJECT_IDENTITY` | Broker-provided canonical file identity; required for children, invalid on a non-child, replacement fails before opening work. `kassiber/cli/main.py:6859`. |
| `KASSIBER_OPERATOR_EXPECTED_DATABASE_IDENTITY` | Broker-provided durable `database_instance_id`; required for children, verified against the opened database before migrations/work, including no-bootstrap secret commands. `kassiber/core/runtime.py:379`, `kassiber/cli/main.py:6895`, `kassiber/secrets/sqlcipher.py:259`, `kassiber/secrets/cli.py:77`. |
| `KASSIBER_OPERATOR_WORKING_DIRECTORY` | Broker sets admitted caller cwd; absent leaves cwd unchanged, non-child usage rejected, vanished cwd fails without fallback/output redirection. `kassiber/cli/main.py:6830`, `kassiber/operator/runner.py:82`. |
| `KASSIBER_NATIVE_AUTH_HELPER` | Required macOS signed helper path, expanded/canonicalized and executable; no fallback elsewhere. Desktop CLI supervisor/launcher sets it; broker inherits it. Signature, bundle location and sidecar identity must match, not just filename. `kassiber/operator/native_auth.py:288`, `kassiber/operator/native_auth.py:241`, `ui-tauri/src-tauri/src/supervisor.rs:1113`, `ui-tauri/src-tauri/bin/kassiber:80`. |
| `APPIMAGE`, `APPDIR` | AppImage build identity uses the actual AppImage file rather than its transient mount when that binding can be proved; otherwise executable/package identity. Runtime/packaging supplies them. `kassiber/operator/build.py:44`. |
| `SSL_CERT_FILE` | Python/OpenSSL implicit trust input. Frozen entry preserves explicit nonempty value, then Linux loadable host bundle list, then bundled certifi. Bad explicit input is not silently replaced. Users/launcher/tests. `scripts/kassiber_pyinstaller_entry.py:25`. |
| `KASSIBER_HOST_CA_BUNDLE` | Internal launcher output: clears inherited marker; Linux explicit SSL override → `explicit`, auto host bundle → `1`, certifi fallback → absent. Observer routing checks only `explicit` because BDK/LWK do not honor SSL_CERT_FILE. `scripts/kassiber_pyinstaller_entry.py:28`, `kassiber/core/chain_observer/host_trust.py:17`. |
| `PYINSTALLER_RESET_ENVIRONMENT` | Frozen broker/child re-exec sets `1` to force independent extraction lifetime. Disappears with PyInstaller; preserve independent child lifetime intent. `kassiber/operator/launcher.py:40`. |
| `TERM`, `NO_COLOR`, `CLICOLOR` | Update-banner color requires TTY, TERM not `dumb`, NO_COLOR absent (even empty disables), CLICOLOR not `0`. Default TERM empty. Shell/user/tests. `kassiber/update_check.py:504`. |

Implicit library/child inputs also matter: `PATH` selects Node, age/rage,
lightning-cli and Poppler (`kassiber/ai/broker_client.py:95`,
`kassiber/backup/age_cli.py:63`, `kassiber/core/document_import.py:465`);
`TMPDIR`, `TEMP`, `TMP` feed Python's temporary-directory fallback when no
verified RAM root is selected (`kassiber/ai/broker_client.py:178`);
`SSL_CERT_DIR` participates in OpenSSL's default verify paths. Locale variables
`LANG`/`LC_*` can affect inherited subprocesses/terminal streams; the backend
does not call `setlocale` to select accounting or CSV formats. Peer validation's
`loginctl` child explicitly fixes `LC_ALL=C` and PATH (`kassiber/operator/peers.py:83`).
Python/keyring/provider dependencies may read their own environment; these are
not new Kassiber configuration or consent surfaces.
Ambient HTTP_PROXY/HTTPS_PROXY/ALL_PROXY/NO_PROXY values do not authorize a new
route: provider HTTP and update checks explicitly disable urllib's environment
proxy discovery (`kassiber/ai/client.py:1791`, `kassiber/update_check.py:82`).

`KASSIBER_TEST_NO_EGRESS`, `KASSIBER_TEST_TAX_ENGINE`, tax recording/module
variables, `KASSIBER_INTEGRATION` and `KASSIBER_CI` belong to test harnesses,
not backend switches (`tests/conftest.py:47`, `tests/tax_engine_compare.py:36`,
`tests/_child_hooks/sitecustomize.py:1`). The Python socket guard allows
loopback and cannot prove native transports are quiet. `KASSIBER_TEST_CLI` is
the planned harness switch, not an existing product getenv at this base.
`KASSIBER_VERSION` is an imported version alias, not an environment read
(`kassiber/core/chain_observer/bdk.py:12`).

Desktop/development selection knobs are not read by the Python backend:
`KASSIBER_PYTHON` and `KASSIBER_REPO_ROOT` choose supervisor executable/cwd
(`ui-tauri/src-tauri/src/supervisor.rs:1740`, `:1777`);
Vite also reads build version/commit and regtest demo home
(`ui-tauri/vite.config.ts:50`, `:1142`, `:1429`). These must be considered when
replacing Python discovery, not accidentally added as book settings.

## State, config and preference files

“Machine-wide” means shared by this OS user across projects and processes.
Linux state is absolute XDG_DATA_HOME/kassiber or ~/.local/share/kassiber;
macOS is ~/Library/Application Support/at.bitcoinaustria.kassiber;
Windows is LOCALAPPDATA/at.bitcoinaustria.kassiber. Existing movable state in
~/.kassiber wins until a safe default-root migration completes. That migration
uses an atomic same-filesystem rename, never copy/delete/merge, preserves
bin/run in the old home, checks owners and rejects unsafe symlinks or conflicts;
active owners defer it (`kassiber/db.py:63`, `:100`, `:343`, `:387`).
Legacy flat ~/.local/share/kassiber and ~/.local/share/satbooks remain fallbacks
when the requested default data root is absent (`kassiber/db.py:3063`).

`--project` and `--data-root` are mutually exclusive. Otherwise selected/default
catalog project supplies data/. An explicit env-file wins over derived config
paths. A data root named data has its parent as state root; flat legacy roots
are their own state roots. Prefer kassiber.sqlite3, then satbooks.sqlite3
(`kassiber/core/runtime.py:82`, `kassiber/db.py:3080`, `:3246`).

| File and scope | Shape, I/O and failure behavior |
| --- | --- |
| global `config/offline-mode.json` | `{schema_version:1, enabled:bool}`, 4096-byte cap; missing means online, existing unreadable/malformed/unknown schema/symlink means offline. `kassiber/egress_policy.py:46`. |
| global `config/update-checks.json` | Consent `{schema_version:1, enabled:bool}`, 1024-byte cap; exact integer schema (bool rejected), missing/unsafe/invalid means disabled. Environment disable wins. `kassiber/update_check.py:192`. |
| global `config/update-check.json` | Cache schema 1: latest_version, release_url, prerelease, checked_at. 8192-byte cap, validates version/trusted release URL/current channel/timezone; invalid is no cached result. 20h cache interval is distinct from desktop's 24h scheduling. A cache is not consent. `kassiber/update_check.py:376`, `:395`, `:435`. |
| global `config/agent-access.json` | Schema 1, mcp_enabled bool and ai_features_enabled bool/null; 1024-byte cap. Invalid/missing disables MCP. Both explicit MCP opt-in and absence of a false AI master switch are required. CLI-only installation may have null master decision. `kassiber/agent_access.py:37`, `:63`. |
| global `config/projects.json` | Schema 1, selected_project_id/null, projects array of id/name/path/encrypted/last_opened_at. Invalid entries skipped; unreadable/invalid JSON/object → empty catalog. Load normalizes schema; no strict source version validation. Temp sibling + flush/fsync + replace + directory fsync (`F_FULLFSYNC` on macOS); best-effort parent 0700/file 0600; no shared read-modify-write lock. Read follows symlinks; write replaces final destination. `kassiber/projects.py:176`, `:270`, `:293`. |
| project `config/settings.json` | Schema 1, app=kassiber, paths manifest (state_root/data_root/database/config_root/settings_file/env_file/exports_root/attachments_root), plus preserved top-level preferences. Unreadable/non-object → `{}`. Sibling settings.json.lock uses flock exclusive or Windows byte locking; same-directory mkstemp, UTF-8 sorted/indented JSON + newline, flush/fsync, replace, then directory fsync (`F_FULLFSYNC` on macOS). Temp is 0600; no blanket 0700 enforcement for existing parents and no private-file/symlink reader. Do not claim it has consent-file hardening. `kassiber/db.py:3113`, `:3125`, `:3163`, `:3214`. |
| project `config/backends.env`, legacy `config.env`, `.env` | Explicit --env-file first; preferred existing config/backends.env, then state-root config.env then .env, otherwise preferred new path. UTF-8 key=value lines, blank/comments ignored, matching outer quotes stripped, malformed lines ignored, no shell expansion. No generic owner/symlink/size validation at load. Plaintext metadata; secret-shaped fields migrate to encrypted DB, never treat it as encrypted. `kassiber/backends.py:255`, `:277`, `kassiber/secrets/credentials.py:29`. |
| root `.state-root-migration.json` | `{schema_version:1,migrated_from_hidden_home:true}`, owner-only, no private paths. Migration reader rejects nonregular final files, writes use fsync + replace + directory fsync; marker travels inside atomic root rename. `kassiber/db.py:129`, `:148`, `:173`. |
| project `config/auth_backoff.json` | `{failures,locked_until}` with UTC epoch deadline, shared across daemon/broker restarts. Three failures start exponential 5s backoff capped at 30s; loaded deadlines are clamped. Missing/unreadable/invalid JSON keeps in-memory state; writes use fixed .json.tmp + replace, no private-file permission/symlink/fsync policy, OSError ignored. Rust also syncs the temporary file and the directory (stricter). It smooths retries, not an adversarial authentication boundary. `kassiber/secrets/auth_backoff.py:20`, `:83`, `kassiber/operator/service.py:1876`, `kassiber/daemon.py:18815`. |

The three small preference files and update cache share
`atomic_write_private`: create parents with 0700, same-directory mkstemp 0600,
UTF-8, flush/fsync then replace; final symlink is replaced, not traversed.
The reader uses lstat + O_NOFOLLOW where available, O_NONBLOCK, fstat regular
file check and limit+1 bounded read. It does **not** check read-side ownership
or mode, or forbid every symlinked ancestor. Existing parent chmod is best
effort for offline/update consent, not a universal rule
(`kassiber/private_files.py:17`, `:51`, `kassiber/egress_policy.py:70`).

Managed settings hold operator_unlock_mode (`manual`, `brokered`, `unattended`),
operator_policy_project_identity and operator_policy_database_identity.
Missing mode defaults manual; invalid explicit mode errors; reusable mode with
missing/mismatched authenticated binding becomes manual. CLI remembered unlock
also needs cli_remembered_unlock and cli_remembered_unlock_enrollment;
legacy quarantine and desktop invalidation flags preserve revocation
(`kassiber/operator/modes.py:25`, `:43`, `kassiber/operator/policy.py:16`,
`kassiber/secrets/unlock_store.py:31`). These files are outside SQLCipher.
Keyring services are `Kassiber CLI Database Passphrase`, read-only migration
`Kassiber Database Passphrase`, and `Kassiber Desktop Biometric Invalidated`.
CLI account is SHA256 of `kassiber-cli-remembered-unlock-v2:<database-id>:<enrollment-id>`
(`kassiber/secrets/unlock_store.py:68`). Preserve desktop's separate namespace.

Runtime layout creation makes the env file's parent, not a new backends.env.
Credential migration first copies a timestamped `.pre-credentials-migration-*.bak`
then rewrites the original dotenv in place, preserving non-migrated lines;
there is no atomic private-file writer in that path. Preserve this observed
distinction in the compatibility review (`kassiber/core/runtime.py:132`,
`kassiber/secrets/credentials.py:178`, `:302`).

## Operator children, locks and identity

Unix endpoint is account-home ~/.kassiber/run/operator-v1.sock, startup election
uses operator-v1.start.lock. Directory must be real, owner-only 0700 and owned
by the user; socket removal checks original inode. Linux/macOS peer uid is
verified; Windows SID-derived local pipe and current-user-only DACL are the
equivalent boundary (`kassiber/operator/protocol.py:34`, `:218`, `:250`,
`kassiber/operator/peers.py:1`). The protocol is version 3 despite the v1
filename: !cI framing, 8 MiB JSON, 16 KiB secret frame, one-use challenge;
never put secrets in JSON or env (`kassiber/operator/protocol.py:25`).

Project identity resolves directory/database aliases and rejects unsafe files,
foreign owners and multiple hard links. Existing file identity material is
`file:<st_dev>:<st_ino>`, absent file `path:<canonical database>`; hash input is
`kassiber-operator-v1:<platform>:<uid-or-SID>:<material>`, public id first 16
hex characters (`kassiber/operator/project.py:204`, `kassiber/db.py:3258`).
Persistent owner namespace ignores HOME, XDG, endpoint overrides: POSIX account
home/.kassiber/run/operator-owners; Windows shell LocalAppData/Kassiber/run/owners.
It holds identity-<hash>.lock, path-<sha256(canonical-db-path)>.lock, plus local
data/.operator-owner.lock. Each group has .broker/.desktop role and .admission
locks (`kassiber/operator/project.py:232`, `:329`, `:574`).

Acquire compatibility shared + role exclusive nonblocking locks at all three
locations under the admission mutex; opposite role must match identity. POSIX
flock shared descriptions and Windows share-mode reservations retain exclusion
through inherited duplicate handles, even if the broker exits. Release closes
handles rather than explicitly unlocking. Lock records include owner, pid,
generation and identity; directory 0700, regular lock files 0600, O_NOFOLLOW
where supported. A second broker or desktop fails project_in_use; one of each
may coexist. Rekey reserves the opposite role before closing/rekeying anything
(`kassiber/operator/project.py:49`, `:142`, `:249`, `:474`, `:645`).

Admission canonicalizes argv data root and pins declared workspace/profile
scope. Recheck filesystem identity before dispatch/launch/bootstrap and the
durable database_instance_id from the actually opened SQLCipher connection
before migration or administrative work. No-bootstrap handlers also authenticate
and close a preflight connection. The child drains its lease pipe eagerly;
Unix pass_fds and Windows explicit handle_list include only required secrets
and duplicated owner handles (`kassiber/operator/service.py:1450`,
`kassiber/operator/runner.py:21`, `kassiber/cli/main.py:6859`, `:6923`,
`kassiber/core/runtime.py:379`, `kassiber/db.py:3554`).

Launch in the trusted installed package directory, then enter admitted caller
cwd once modules have resolved; identity checks still precede runtime bootstrap; caller no-egress can only tighten inherited policy.
Feed secret pipes concurrently while draining stdout/stderr to avoid capacity
deadlocks (`kassiber/operator/launcher.py:24`, `kassiber/operator/runner.py:55`,
`:105`). Passphrase fd input is capped at 8192 bytes, removes one trailing LF
then one CR, decodes strict UTF-8, preserves other spaces and closes the fd;
Windows broker children convert inherited raw handles
(`kassiber/secrets/prompt.py:24`, `:48`, `:93`).
Accepted mutations have no arbitrary total timeout. Generation ids,
bounded terminal caches/tombstones and result_unknown avoid false exactly-once
claims after crashes (`kassiber/operator/service.py:175`,
[queue contract](operator-broker.md#queue-and-operation-semantics)).

## Catalog, wire records and errors

`commands describe` walks argparse leaves and action order, not a hand-maintained
list. It derives path/kind/help/arguments, capability and effect, database need,
scope flags, cursor/dry-run support and possible prompting. Secret-shaped dest
names mark arguments; the capability exact-path registry is independent of
the parser and fails closed. Catalog database need is conservative: runtime
exceptions include wallets.import-ledger --dry-run and btcpay.key-url without
a backend (`kassiber/cli/command_registry.py:20`, `:62`, `:97`, `:122`, `:180`,
`kassiber/command_capabilities.py:1`).

Catalog kind is the full dotted path; output derive_kind walks an ordered list
of namespace attrs and supports explicit overrides. projects, records-tag/
excluded/kind and filed-snapshot subcommands require path-only handling; full
catalog kind and emitted kind need not coincide. json_ready converts Row/dict/
list recursively and Decimal to float; it is not the serializer used for all
stored hashes. Normal JSON output is indented, ensure_ascii defaults true,
unsorted keys and trailing newline. Pagination adds page only with both
next_cursor and has_more, unless page already exists; envelope_meta cannot
override kind/schema_version/data/error (`kassiber/envelope.py:41`, `:64`,
`:132`, `:150`, `:164`, `:286`). Usage errors in machine mode exit 2 on stdout;
broker admission events are on stderr, not stdout
(`kassiber/cli/main.py:61`, `kassiber/operator/cli.py:250`).

Supported daemon requests are not identical to emitted record kinds. Exact
request-id/exact-kind terminal or error completes a request; other records on
that id are streaming/control. Events have event:true and no request_id;
daemon.ready is the startup exception. Secret-store control requests/replies
have their own protocol. Frozen fixture includes dynamic construction sites
alongside literal ai.chat.delta, progress and background events
(`kassiber/daemon.py:380`, `:1509`, `:9047`, `:18830`,
`kassiber/envelope.py:164`, `ui-tauri/src-tauri/src/supervisor.rs:1`).
Both ordinary invoke allowlists have 283 entries and are equal/subsets of
daemon support. Tauri streaming has 15, Vite has 14: ui.source_funds.assemble
is absent from Vite streaming. ai.chat is Vite stream-only. AI gates are a
separate authority decision (`ui-tauri/src-tauri/src/lib.rs:176`, `:480`, `:518`,
`ui-tauri/vite.config.ts:96`, `:392`, `:408`). Do not silently make these lists
equal or infer invoke permission from a supported daemon kind.

Error code alone is not a retry policy: inspect each construction site in the
fixture. AppError defaults app_error / retryable:false; builders normalize
retryable with bool, and transport/broker errors may compute it from context.
Bare issue dictionaries can omit retryable altogether. Dynamic codes and
wrapper forwarding are indexed as expressions, with the defining source;
the Rust error type must preserve these distinctions rather than assign one
global retry flag per code (`kassiber/errors.py:27`, `kassiber/envelope.py:177`).

## CSV, encoding and locale

- General importer CSV uses UTF-8, newline="", comma DictReader; BOM stripping
  is opt-in per importer, not universal. Coinfinity, Ledger Live and Binance supplemental
  explicitly use UTF-8-sig (`kassiber/importers.py:85`, `:1441`, `:2163`, `:2347`).
- Generic ledger CSV uses UTF-8-sig, samples first 8192 characters for comma,
  semicolon, tab or pipe dialect, falls back to standard CSV; recognizes a
  header after preamble rows. Generic numeric parsing accepts comma or dot
  decimal, but that is not a global locale switch
  (`kassiber/importers.py:3505`, `:3523`, `:4382`).
- File analysis previews decode UTF-8-sig with replacement and bounded samples,
  then sniff delimiters. This permissive preview does not imply every actual
  importer accepts malformed UTF-8 (`kassiber/core/file_analysis.py:80`).
- Ownership CSV text strips BOM, sniffs the same four delimiters and recovers
  tokens after malformed/oversized fields using raw splitting; keep bounds and
  warnings (`kassiber/core/ownership.py:652`, `:691`).
- CLI --format csv and report CSV write UTF-8 without BOM using csv.writer/
  DictWriter with standard Excel dialect (comma, minimal quoting, CRLF),
  newline="" prevents extra CR insertion. CLI headers follow first-row key
  order, extra keys ignored; None → empty, dict/list → JSON, floats → up to
  eight places stripped of trailing zeros. Output '-' is stdout; files expand
  ~ and create parents (`kassiber/envelope.py:193`, `:220`, `:241`, `:335`,
  `kassiber/core/reports.py:5493`, `kassiber/importers.py:4397`).
- CLI/daemon help/errors are English. Austrian report copy is German across
  CSV/XLSX/PDF, including Kennzahl labels and unsupported placeholders;
  tax-profile generation_language remains en. Decimal/cent formatting is
  explicit, independent of LANG: report fiat uses comma grouping and a dot
  decimal (`1,234.56`), despite the German copy. Preserve each renderer's format and PDF
  transliteration/font behavior (`kassiber/core/reports.py:1901`, `:4452`,
  `:4863`, `kassiber/tax_policy.py:26`, `kassiber/_pdf_common.py:1`).

The inventory confirms plan 21's command/kind counts. Verification item 3 also
needs the fifth, Python-only disposition: these tests retire with Python but
their security/packaging intent needs the named native replacements. Static
inventory coverage is not a native-parity completion metric.
