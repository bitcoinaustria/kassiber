# Rust book runtime (R1/R3)

Python remains the reference. The Rust `kassiber` binary now uses these
crates for `init`, `status` and the daemon's supported book lifecycle, including
SQLCipher fd unlock, bound remembered unlock, TTY fallback, compatible ownership,
default state-root cutover and legacy project migration. It never invokes Python.
The test-only Cargo probes are not installed or included in release artifacts.

The ordered pass runs the custody migrations it cannot evaluate itself
(legacy review consolidation, the legacy pair fee review, componentless
dismissal recovery) through the custody layer the binary installs; see
[legacy custody migrations](database-compatibility.md#legacy-custody-migrations-in-the-rust-store).
The supported ordered pass and legacy project migration are now selected
explicitly by shell bootstrap; `Book::open` retains admission-only behavior.
TODO.md ("Rust backend R3 storage") tracks the remaining durability and native
platform gates. The [shell reference](rust-shell.md) records the binary contract.

## Public API for CLI and daemon callers

- `kassiber_paths::Context::from_env()?.resolve(&ResolveOptions)` resolves
  `RuntimePaths`. `ResolveOptions` has `data_root`, `project`, and `env_file`;
  the first two are mutually exclusive. `Context` accepts an explicit
  platform, home and environment for deterministic tests. Its
  `native_state_root`, `default_state_root`, `effective_data_root`, and
  `state_root_for_data` mirror Python's read-only resolvers.
- `kassiber_ownership::OwnerNamespace::system()` selects the fixed OS-account
  rendezvous. `canonical_project(data_root)` returns the canonical database,
  filesystem identity, public id, and identity/alias/local lock paths.
  `acquire(project, OwnerKind::{Desktop,Broker}, generation)` returns an RAII
  `OwnerLease`; `verify_current` rechecks replacement, `add_alias` retains
  additional paths, and `duplicate_for_child` returns non-inheritable `File`
  copies. Only the broker launcher marks its selected copies inheritable.
  `bind_created_database` adds a new inode reservation while retaining the
  original path reservations after a first open creates the database. It
  refuses replacement of a file that existed when the lease was acquired.
  `OwnerNamespace::isolated` is explicit test/embedding injection, never an
  environment-selectable product namespace.
- `kassiber_store::Book::open(data_root, OpenOptions)` opens an existing book.
  Options include `passphrase`, `expected_database_identity`, and
  `expected_project`. The book exposes `instance_id`, `encrypted`, `database`,
  `schema_fingerprint`, and its trusted `rusqlite::Connection`. Pass the
  lease's project in every long-lived open, including locked-state retries.
- `Book::open_migrating(data_root, options, require_existing_schema)` is the
  production shell's explicit open/create/compatibility API. It verifies supplied
  file/database bindings before writes, moves the book to its safe journal
  mode, runs the ordered pass, then applies the structural/semantic gate.
  New/empty schemas use the transactional creation transcript; existing
  schemas never replay it. The pass's custody steps need the installed custody
  layer (`kassiber_store::custody_hooks`); without it a book needing one is
  refused before the step writes.
- `Context::resolve_for_bootstrap` uses the same resolver without demanding an
  already-published layout. Shell bootstrap calls `projects::ensure_default`
  followed by `layout::initialize_layout`; after unlock it calls
  `projects::validate_after_unlock`. Resolution itself remains read-only.
  Legacy copying refuses existing held local owner reservations and nonempty
  WAL/rollback journals instead of copying a source with pending pages. The
  complete concurrent-copy matrix remains an R3 acceptance gate.
- `restore::resolve_for_bootstrap(context, namespace, options)` locates the
  selected container without admitting its settings, recovers a pending restore
  under both maintenance roles, then resolves again against the recovered
  configuration. The binary uses this before layout, unlock-policy and local
  secrets/backup reads. `Book::open` and `layout::initialize_layout` refuse a
  pending restore instead of creating or migrating through it.
- `restore::maintenance(namespace, data_root, active_owner)` retains both path
  and file reservations across database replacement. Supply an active role only
  while retaining that role's existing lease. With connections closed and these
  reservations held, `restore::install(staging, data_root)` publishes the shared
  Python/Rust transaction; `restore::recover` replays it without opening SQLite.
  Pending transactions roll back, committed transactions retain the new tree
  and original `pre-restore-*` copy. See the [backup continuation](rust-shell.md#backup-install-and-recovery-continuation-r5)
  for its durable-step matrix and process-interruption limits.
- `kassiber_secrets::UnlockSession::prime_fd(fd)` consumes/closes an explicit
  descriptor once; `cache(Passphrase)` also accepts an authenticated daemon
  control-channel value. `open(data_root, UnlockOptions, CredentialReader,
  callback)` tries cached/explicit input, an unkeyed open, bound unattended
  remembered unlock, then a controlling-terminal no-echo prompt when stdin
  is a terminal and `non_interactive` is false. Otherwise it returns
  `interaction_required`, with `details.reason = database_passphrase`.
  The callback receives `(Option<&str>, Option<&str>)`: passphrase and the
  expected database ID, both of which must be passed to `Book::open`.
  `NativeCredentialReader` constructs stores only on an authorized lookup.
  `take_notices` reports a stale remembered credential for stderr handling;
  no library writes machine stdout or creates a log file. A remembered cache
  retains its database binding on reopen. Clear/drop the session on project
  switch or lock; it belongs to one authorization session, not the whole app.
- After a successful unlock, `OwnedBook::from_open_book(book, lease)` retains
  the lease and guarantees connection-before-lease drop order. For an
  already supplied passphrase, `OwnedBook::open(lease, options)` combines
  the two. `book_mut().transaction(callback)` starts `BEGIN IMMEDIATE`,
  commits on success, and rolls back on error or unwind. Use
  `with_savepoint(transaction, callback)` for nested work.
  Runtime retirement calls `OwnedBook::close`; a failure returns
  `CloseFailure { resource, error }` retaining both the connection and its
  lease for retry. `Book::close` similarly preserves a failed connection.
  Implicit RAII drop is a teardown fallback, not a retirement retry queue.
- All crates share `kassiber_paths::{Error, ErrorKind, Result}`; store and
  secrets re-export them. Map `error.code`, `message`, optional `hint`, `details`, and
  `retryable` directly into the shell's envelope. Match `ErrorKind` for
  control flow. `map_sqlite_error` maps base and extended BUSY/LOCKED to
  `database_busy` with `retryable=true`; safe-journal failures retain
  Python's separate `database_journal_mode_unavailable` code. No raw SQL or
  credential-bearing driver messages enter details.

A shell retains the lease across unlock attempts, not just the final open:

```rust,ignore
let context = kassiber_paths::Context::from_env()?;
let paths = context.resolve(&selection)?;
let namespace = kassiber_ownership::OwnerNamespace::system()?;
let project = namespace.canonical_project(&paths.data_root)?;
let lease = namespace.acquire(project, OwnerKind::Desktop, &generation)?;
// prime_fd, if supplied, happens once before dispatch, including broker pipes.
let book = session.open(
    lease.project().database.parent().unwrap(),
    UnlockOptions { non_interactive: true, expected_database_identity: expected_id },
    &NativeCredentialReader,
    |passphrase, expected_database_identity| Book::open(
        lease.project().database.parent().unwrap(),
        OpenOptions {
            passphrase, expected_database_identity,
            expected_project: Some(lease.project()),
        },
    ),
)?;
let runtime = OwnedBook::from_open_book(book, lease)?;
```

Short-lived CLI operations do not impersonate a desktop or broker role;
SQLite still arbitrates their writes. Database-wide administrative work must
reserve both roles (or the role opposite its existing owner), as Python does.
The R5 secrets adapter uses those leases for explicit encryption initialization
and passphrase rotation. Its authenticated native lifecycle and dotenv credential
move are recorded in the [secrets continuation](rust-shell.md#secrets-lifecycle-continuation-r5).

## Read-only resolution and initialization barriers

Linux uses absolute `XDG_DATA_HOME/kassiber` or
`~/.local/share/kassiber`; macOS uses
`~/Library/Application Support/at.bitcoinaustria.kassiber`; Windows uses
absolute `LOCALAPPDATA/at.bitcoinaustria.kassiber` or its home fallback.
Only the environment inputs Python actually reads are honored; there is no
invented `KASSIBER_DATA_ROOT` or state-root override. Explicit `--env-file`
and legacy `config.env` / `.env` precedence match Python.
Meaningful hidden-home state includes unknown top-level entries, excluding
`bin`, `run`, and `.state-root-migration.json`. Flat Kassiber/Satbooks
locations, the `data` basename rule, and `satbooks.sqlite3` fallback remain.

Resolution never creates directories, selects a project persistently, updates
last-opened/encrypted metadata, writes settings, or migrates roots. An absent
catalog/default project or incomplete runtime directory/settings layout yields
typed `not_initialized`. A hidden-home root (`~/.kassiber`) the cutover left
in place (a symlink, a link inside it, another file system, an invalid proof)
or that the caller skipped (`--env-file`, `backup import`) stays the state
root, as in Python's `default_state_root`. A project-layout migration or a
conflicting migration proof yields `requires_python_migration`. An explicit
data root never migrates and wins over the default root's catalog: that
catalog only names the explicit root's project, and one the reader refuses
names nothing instead of blocking the command. Those are read-only resolver
barriers. Explicit shell bootstrap now uses the creation, ordered-pass
and project APIs above; it never treats an existing rejected book as fresh.

Catalog and settings readers accept the Python schema-1 object shape and
normalize valid project ids/names like Python. The deliberate R1 conservative
boundary refuses malformed metadata, unknown schema versions, symlinked files
or their immediate config directory, nonregular files, another owner, and
POSIX group/world writes. User-owned `0600` and `0644` settings and catalog
metadata are admitted, matching Python-written books; `0664`/`0666` and
foreign owners remain refused. Reading does not change permissions. Rust's
own new settings and metadata writes retain `0600`. Missing settings are empty authority, never
remembered-unlock permission. Python's general loaders sometimes silently
return empty data or normalize older/unknown metadata; Rust refuses those
inputs instead of guessing or replacing them. It also bounds metadata at
8 MiB. These refusals require recovery through Python; they do not change its
loaders. Saved manifest paths do not override computed routing.

Machine preferences use a separate contract: `read_small_regular_file` bounds
the read and refuses symlinks and special files without blocking on FIFOs.
It does not apply settings/catalog ownership and mode admission. The caller
decides whether missing/unreadable bytes mean consent refused or offline mode
enabled. Explicit `layout::write_private_preference` publishes a private,
flushed temporary file by atomic replacement; like Python's preference writer,
it replaces a final symlink without following it and sets the parent to `0700`
where supported. Book metadata retains its stricter storage path checks.

## Existing database, schema gate and durability

`Book::open` never sets `SQLITE_OPEN_CREATE`. It checks the exact 16-byte
`SQLite format 3\0` header, rejects hard-link ambiguity, and opens plaintext
or SQLCipher accordingly. An explicit passphrase on plaintext returns
`plaintext_database`; an encrypted file without one returns
`passphrase_required`; a wrong key is `unlock_failed` (retryable), while
contention is `database_busy`. Missing/zero-byte files are `not_initialized`.

SQLCipher applies a quote-doubled SQL string literal as `PRAGMA key`, then
`cipher_compatibility=4`, `kdf_iter=256000`, and `cipher_page_size=4096`, before
reading `sqlite_master`. Empty/NUL values are refused. The key literal and
application-owned input/cache buffers are zeroized on drop, including invalid
UTF-8 and native-store decode error bytes. SQLCipher, terminal and OS-store
libraries also own transient buffers; this is not an mlock or process-memory
secrecy claim. Input is limited to 8192 bytes before removing one LF then one
CR, and descriptors close on success and every failure. On Windows,
`read_passphrase_from_fd` accepts CRT fds; broker launchers must instead pass
an `OwnedHandle` to `read_passphrase_from_inherited_handle` for OS handles.

Before changing journal mode, the connection verifies `database_instance_id`
(32 lowercase hex digits) and any supplied/operator-child expected binding.
Missing child bindings fail closed. The filesystem identity is rechecked
around the gate when `expected_project` is supplied. The admission-only
`Book::open` runs no migration, generated identity or DDL.
`Book::open_migrating` deliberately selects those supported operations after
binding/authentication and before the final gate.

The checked-in `crates/store/fixtures/current-schema.json` freezes every
non-internal `sqlite_master` table, index, trigger and view with its full SQL
(including column types, defaults, constraints, foreign keys and partial
indexes), after Python's current `open_db`, including graph-index installation.
When the complete non-internal `sqlite_master` object list matches the frozen
definitions byte for byte, the gate accepts that identical structure directly.
Otherwise it compares normalized structure using `table_xinfo`, `index_list`,
`index_xinfo`, grouped `foreign_key_list`, tokenized trigger/view/index SQL,
and table constraints. ALTER-appended column order, comments, identifier quotes
and insignificant whitespace do not cause refusal. String literals remain
exact. It recognizes Python's historical nullable `transaction_edit_events`
sync columns and its two ALTER-added custody-count columns lacking fresh-book
CHECK clauses; all other constraints remain compared. Both historical fixtures
pass after Python migrates them twice. The gate validates all five mandatory
migration-audit id/name/version triples, and accepts only the two known
additional historical audit names. Impact payloads and timestamps are book
specific and are not compared with a fresh book's values. A read transaction
keeps the gate's schema and semantic reads consistent. The public SHA-256
schema fingerprint hashes the compact JSON object list; the audit gate is
validated separately.

Current-book compatibility passes compare existing replacement index, view and
immutability-trigger definitions before issuing DDL. Matching definitions avoid
schema writes and close-time WAL checkpoints; changed or missing definitions
still run their original repairs. Ordered column ranges reuse each table's
column inventory within the range. Liquid worklists first test the same asset
predicates against a materialized distinct-asset inventory using existing
covering indexes. No index, migration marker or schema version is added, and
the final structural, audit and semantic admission checks still run.

`semantic-probes.sql` and the durable-review relation checks refuse pending
source-link/target snapshots, provider names, ownership-history cleanup,
wallet-scoped labels, legacy custody consolidation, anchors/evidence,
payment hashes, stale same-asset fees, and gap-review backfills. Guards follow
Python's write predicates: only the three known Liquid policy-asset IDs are
rewritten; Liquid identity backfill requires a Liquid wallet, a valid txid and
noncontradictory evidence, never touches a graph whose observer provenance
still matches, and is pending only for a graph an earlier backfill rewrote and
the committed hash can restore; payment hashes must be eligible top-level strings;
inline ownership history must be a list under the exact key. Python does not
repair a missing graph-clock row when its tables already exist. Unrecoverable
dismissals in books without observations are admitted. Candidate recovery with
observations still requires the unported custody matcher and returns the
distinct `unsupported_schema_layout`, rather than claiming another Python open
will repair it. Pending authored migration issues also remain refused.
Populated removed ledgers and unknown
column/constraint changes still fail the structural gate.
R3 must broaden acceptance only with parity evidence;
R1 is not a claim to open every historically migrated layout.

Strict JSON parse failures never become empty migration inputs. Stored Python
extensions (NaN, Infinity, overflowing floats, lone surrogates) return
`unsupported_schema_layout` until a compatible reader exists, preserving every
byte. Admission also refuses integer literals outside signed/unsigned 64-bit
range instead of rounding through f64, and nonempty collections in normalized
identity fields whose Python string representation is unported. Migration
normalization includes Python's U+001C–U+001F strip characters
and the SQL Liquid spelling predicate, including `LBTC-` and `L-B-T-C`.

After admission, `busy_timeout=30000`, `temp_store=MEMORY`,
`cache_size=-16000`, and `foreign_keys=ON` match Python. Encrypted connections
explicitly disable mmap; plaintext requests 268435456 bytes. While Python can
still open books, Rust never promotes an existing database to WAL. SQLCipher
uses DELETE/FULL because the shipped Python `sqlcipher3` embeds SQLite 3.51.1.
Plaintext retains its existing journal mode; existing WAL uses NORMAL only
when Rust's embedded SQLite has the WAL-reset fix (3.51.3+, 3.50.7+, 3.44.6+).
Otherwise it requires DELETE/FULL. A failed transition is an error.

New books default to conservative DELETE/FULL. Packaging that knows its shipped
Python plaintext SQLite version supplies it to `Book::create_for_python_runtime`
to select the same mode as Python. SQLCipher remains DELETE/FULL throughout
coexistence. Tests hold a Python connection open across Rust opens/writes and a
Rust connection across Python opens/writes, for both book types. Comparing only
the Rust engine's version cannot prove safety for a live Python connection.

`Book::hot_backup(&mut destination_book)` uses SQLite's incremental online
backup API with a bounded retry deadline, preserves the source database ID,
and refreshes the destination's cached ID. Both books must already exist,
have the same encryption mode, and name distinct canonical files. This API
cannot implicitly decrypt a source. It does not publish a backup archive,
rebind destination unlock settings, create an artifact, or authorize restore.

R3 acceptance requires creation, exact compatibility and semantic migrations,
legacy state/project migration, rekey, atomic backup/restore publication,
permissions/durability of new artifacts, crash/failure injection, recovery,
and the full storage durability gates in plan 21. Implemented private subsets
and their evidence are described below; R1 interop alone does not establish
complete migration or production cutover readiness.

## Storage roots and symlinks

Python resolves the roots a user or the OS chooses (`Path.resolve`): a data
root, its state root, the app-data directory, the legacy `~/.kassiber` root
and an explicit `--env-file` directory. Links above them are the user's
layout, such as macOS `/var` -> `/private/var` (where `$TMPDIR` lives) or a
linked home or volume. The store does the same with `durable::under(root,
path)`: the root resolves, the Kassiber-managed tail stays lexical, and
`durable::refuse_symlinks` still refuses every link from the root down. A
data root that is itself a link resolves, as in Python. Envelopes and the
settings manifest keep Python's lexical paths.

Below the resolved root the store is stricter than Python, which follows
these links: the database file, its SQLCipher candidate and rollback backup,
the settings file and lock, the `config`, `exports` and `attachments`
directories, catalog, auth backoff and project migration markers, and the
migration and legacy owner locks. A link swapped in for any of them could
otherwise redirect a file Kassiber creates or replaces. The refusal is
`unsafe_configuration` (`storage path contains a symlink`); metadata reads
keep `configuration is unreadable or unsafe`. Where Python also refuses, the
envelope matches it: a linked legacy root, or a link inside it, keeps the
legacy layout, and restore links (`.restore-pending.json`, restore targets,
staged sources) answer `unsafe_restore_target` with null details. Restore
destinations are resolved first in both runtimes. Staged sources are not:
`tests/rust_port/repro_restore_staging_symlinked_tmpdir.py` shows Python
refusing a restore staged under a linked `$TMPDIR`, and the store keeps that
refusal until the reference changes.

## Native stores and platform gates

Direct keyring store versions match the desktop: `keyring-core=1.0.0`,
`zbus-secret-service-keyring-store=1.0.0`,
`apple-native-keyring-store=1.0.0`, and
`windows-native-keyring-store=1.0.0`. Linux uses the explicit Secret Service
store without `target` (service/username attributes find Python's items).
macOS uses classic generic passwords in the user keychain, not protected
biometric items. Windows searches bare service first, then `account@service`,
requires the stored username to match, and uses password APIs (UTF-16).
`update_existing_password` is an explicit maintenance operation preserving
existing item identity/attributes; ordinary unlock only reads.
Ordinary startup and display reads do not contact a store. Explicit `secrets
status` checks native availability as Python does and may contact the Linux
session bus; it reads a credential only for a bound, enabled, unquarantined CLI
enrollment. Desktop AI and biometric
namespaces are separate and never used for CLI remembered unlock.
Explicit enrollment, refresh and revocation now create/update/delete the CLI
items. Windows retains Python's enterprise-persistent UTF-16 bare-service and
`account@service` targets, including displacement of a prior bare-service user;
it never selects the desktop AI `account.service` namespace. Deletion verifies
absence instead of interpreting a platform deletion error as success.

Native adapters are optional through `kassiber-secrets/native-keyring`, with no
default features. Core/descriptor-only builds contain no keyring, D-Bus or
Tokio dependency. Product shells enabling unattended native lookup must select
that feature explicitly; disabled builds return `secret_store_unavailable`
and preserve normal local-interaction fallback. The interop probe enables it
explicitly. The pinned desktop-compatible Linux adapter exposes a blocking
API over async internals; switching its runtime flag does not remove that
stack. Isolation preserves its tested item format without replacing the backend.
The storage crates share SHA-256/RustCrypto 0.11 with that adapter. The combined
workspace retains custody's separately pinned 0.10 generation; neither branch's
dependencies were upgraded. No new third-party crate is introduced.

The CLI service is `Kassiber CLI Database Passphrase`, with account
`sha256("kassiber-cli-remembered-unlock-v2:{database_id}:{enrollment_id}")`.
Only unattended mode, a matching canonical file identity, valid database and
enrollment ids, the explicit CLI marker, and no quarantine permit a lookup.
The actual keyed connection must then carry that database ID. Old path-keyed
and shared-service items grant no authority. Manual and brokered modes do
not read a remembered item. Native-store failures never permit file fallback.

`rusqlite=0.40.1` uses bundled SQLCipher, backup and functions. Linux and
Windows select `bundled-sqlcipher-vendored-openssl`; macOS selects
`bundled-sqlcipher` (CommonCrypto, when no build-time OpenSSL override is set).
The two codecs may embed different SQLite releases; the coexistence journal
rule above applies. Build-time crypto overrides need explicit
packaging verification. Native macOS/Windows round trips, Windows share-mode
and SID/file-id parity, macOS ACL/search-list behavior, signed executable
identity, packaged crypto linkage, and signed Touch ID hardware validation
remain platform acceptance gates. Non-default/alternate Secret Service
attribute schemes are not inferred; ambiguous stores fail closed.

Native platform acceptance still requires these specific checks:

| Platform boundary | Required evidence before claiming parity |
| --- | --- |
| Windows broker secret pipes | Inherited OS handles must use `read_passphrase_from_inherited_handle`, not the CRT fd adapter; run the native inherited-handle regression. |
| Windows path aliases | Case-insensitive selection, catalog matching, ownership and expected-project comparisons must agree with Python on NTFS; run the native case-alias regression. |
| Windows file identity | Current volume serial/file-index material targets Python 3.11 and older. Python 3.12+ inode semantics require independent comparison, not an assumed match. |
| macOS device identity | Compare signed `st_dev` with Rust's `dev_t` conversion, including a device value requiring sign extension. Linux positive-device evidence does not cover it. |
| macOS realpath | Compare Python and Rust case normalization on a case-insensitive volume, including differently cased aliases; run the native case-alias regression. |

These tests skip on Linux. Their presence is not evidence of a native pass.

## Verification

### R3 creation work

`Book::create` is an explicit empty-book operation, separate from R1's
existing-book admission. It refuses nonempty schemas and a linked database path, creates
owner-only directories/files, and commits all schema objects, audit markers,
the database UUID, and graph-index initialization in one transaction. A failed
statement rolls the whole schema back; either implementation can finish an
interrupted empty-schema creation. It checkpoints and syncs the database
and parent before returning. `layout::initialize_layout` merges the managed
path manifest under Python's settings lock; `layout::write_catalog` publishes
complete catalogs using a flushed sibling and directory fsync.

Python's DDL remains unchanged. `tools/export_creation.py` records every write
made by Python `open_db` on an empty disposable book, preserving SQL whitespace.
The generated `fixtures/creation.json` is valid **only on an empty schema**;
it must never be replayed on an existing book. Its drift test fails if any
empty-book write changes. `test_rust_store_migrations.py` compares full schema
objects and all table rows before and after Python reopens Rust-created
plaintext/SQLCipher books. Identity and SQLite audit time are injected into
both implementations before execution; nothing is masked from their dumps.
The optional `test-hooks` Cargo feature supplies the development probe and
deterministic sources used by the binary's separate `test-harness` feature.

Storage-only Rust ports currently cover initial-schema transaction/rollback,
all per-statement creation failure boundaries, the private catalog writer,
and atomic settings replacement. The R0 `store` disposition also includes
journal orchestration, network partitioning and prior-tax-report services;
those require their owning service stages. Historical compatibility, semantic
migrations, root migration and encryption lifecycle remain R3 acceptance work.

`semantics::migrate` ports the five projection invalidations in Python order:
wages acquisition, RP2 pool basis, fail-closed custody holds, Kassiber tax engine,
and Austrian acquisition valuation support. It requires the compatibility
columns already installed. Each sentinel and profile update share one immediate
transaction, including on failure. `fixtures/semantics.json` pins the Python
predicates, templates and source hashes; impact JSON uses `kassiber-pyjson`,
including the valuation migration's distinct insertion order and separators.
Tests cover the one-shot/invalidation rules from
`test_wages_semantic_migration.py` and compare complete database
states against Python for each missing marker and all missing markers together.
This helper does not yet constitute a complete compatibility pass.

The continuation adds private executable helpers for retired inline ownership
epochs, Liquid asset/identity/fingerprint repair (using the existing internal
`kassiber-pydecimal` crate for Python's exact decimal strings), complete
swap-schema orchestration, invalid/missing database identities, and the grouped
durable-anchor/evidence backfill with its exact audit. These helpers stay
internal to the store; production selects the supported ordered pass while
`test_hooks::migrate` / probe operation `migrate` expose deterministic test
sources. Both support keyed SQLCipher and separate initial/valuation
semantic-marker stages.
The deterministic test migration refuses a required journal-mode transition
before writes; product opens make it. `Book::open` still does not migrate,
while `Book::open_migrating` explicitly runs this pass. Authored consolidation,
the legacy pair fee review and componentless dismissal recovery run through
the installed custody layer; both original fixtures migrate to the exact rows
Python wrote, plaintext and SQLCipher. Python's custody relation view follows
the leg-role rebuild, and Rust follows that order exactly.

The private compatibility work now covers the preflight column repair, base
DDL, all five legacy REAL-to-msat table rebuilds and the graph projection
installer, including outer-transaction preservation and physical-rowid dirty
tracking. `fixtures/compat.json` pins Python SQL and helper source hashes;
the Rust conditions remain handwritten. The two legacy starting points built
by `test_msat_migration.py` compare every schema object and table value, then
prove idempotency and a no-change Python reopen. Fault-injection entry points
remain behind `test-hooks`; the production ordered
pass runs every step, the custody ones through the installed custody layer.

Private schema-repair steps additionally cover attachment/provenance and child
foreign-key rebuilds, legacy pair/recipient uniqueness, custody projection
columns/view, review-note backfills, custody immutability/freeze triggers,
direct payouts, AI secret-reference DDL, commercial/freshness/graph-cache DDL
and source-funds target snapshots. Differential tests reuse Python's stranded
attachment and provenance fixtures and authored review-note fixture, compare
all objects/rows and foreign-key findings, and prove repeated execution leaves
the same state. Fourteen killed-child cases resume attachment rebuilds through
either implementation without losing attachment provenance. These helpers
remain private, with no runtime claim of complete historical migration.

Private custody steps now rebuild ordinal-keyed review relations into stable
transaction-set identities, recover the old half-renamed table, backfill
snapshot/leg anchors and completeness headers, and create payload-free evidence
commitments solely from authored activation snapshots. Missing snapshots leave
anchored components unbound; only an active transactionless component receives
a zero-evidence commitment. Python fixture comparisons cover both relation
table shapes and the authored migration-acceptance fixture, with eighteen
killed-child cases resumed by either runtime. Stable ids, snapshot detail hashes
and rows compare without masking. The local iterative JSON decoder preserves
Python's extended numbers, duplicate-key rules, arbitrary integers and surrogate
escapes through `kassiber-pyjson`; malformed JSON and decoder limits remain
distinct, with nesting over 1000 refused explicitly.

The private suspense-role rebuild copies all three custody child tables in
one immediate transaction, preserves counts and authored rows, restores every
scope/immutability/index object, checks child foreign keys before committing,
and restores enforcement on failures. Its isolated step matches Python for
both historical fixtures after Python has run the preceding steps. This
isolated step evidence alone does not establish full original-book migration.
Fault injection and fourteen killed-child cases cover every rebuild boundary
and recovery through either runtime.

Private backfills also consolidate BIP329 labels with Python's latest-non-null
field rules and merged JSON, populate only eligible top-level payment hashes,
clear stale same-asset manual swap fees, preserve inbox rowid pagination holes,
and retain the latest journal gap-input snapshot before dropping retired
derived caches. Tests compare complete database states, including extended
JSON and Python integer conversion for Unicode digits/underscores, and inject
failures before and after each step's transaction boundary. The ordered
production open invokes these supported steps. Authored
consolidation and exact matcher recovery remain refused; the complete historical
and durability acceptance is still unfinished.

The durability harness also holds real Python/Rust connections across writes
in both directions and runs three Rust writers alongside a Python writer on
one book. Plaintext and SQLCipher counters retain every committed increment;
held transactions return busy and succeed after release, with integrity and
foreign-key checks afterwards.

`state_root::migrate_hidden_home` is an explicit rename-only cutover, with the
same prepared `.state-root-migration.json` proof, process lock, fixed `bin`/`run`
restoration, project-catalog rebasing and conflict refusal as Python. It keeps
database inodes, unknown local files and special files, keeps symlinked legacy
layouts legacy (links above the home and app-data roots are resolved),
and never copies or merges roots. Cross-device failures remove a prepared proof
and retain legacy authority. Either runtime resumes interrupted cutovers.
Rust creates missing directories owner-only and syncs each publication boundary;
catalog replacement retains its prior permissions. Python/Rust comparisons cover
the complete source/target tree, case-specific refusals and held Python owners;
killed-child tests cover proof, root rename, fixed paths and catalog replacement.
Legacy flat XDG/Satbooks discovery remains in `kassiber-paths` and never migrates
storage during a display/read resolution.

`retired::drop_device_sync` preserves Python's exact object allowlist, removal
order and `device-sync-removal-v1` audit bytes, committing the audit and drops
together. Repeated calls with no retired objects only inspect the catalog.
`retired::retire_empty_legacy_ledger` drops empty `gl_*` tables and their
document-retention triggers in one savepoint, with children before parents;
populated archives and every guard remain intact. The read-only
`require_legacy_ledger_absent` exposes `legacy_ledger_present` to destructive
service callers. The structural schema check accepts populated archives
(`gl_*` tables and their indexes by name, and only the `gl_*` triggers
Kassiber shipped, matched by name, table and normalized SQL against
`fixtures/legacy-ledger-triggers.json`); any other trigger is drift, so such
books open only in a shape Kassiber itself wrote;
`legacy_ledger_purge_plan` and `purge_legacy_ledger` serve
`maintenance purge-legacy-ledger`. Python comparisons cover all retired sync names/indexes,
audit counts and idempotency, and empty/populated ledger fixtures with foreign
keys and retention triggers; Rust tests cover rollback of audit/drop groups.

`encryption::encrypt_plaintext` acquires both compatible maintenance role
leases, secures DELETE journaling, exports through `sqlcipher_export`, copies
`user_version`/`auto_vacuum`, and verifies a fresh keyed connection against the
entire source schema and every row. It checks integrity and credential markers,
flushes the private sibling, retains the plaintext rollback copy at Python's
filename, and syncs the directory after each rename. Existing artifacts are
refused. `find_resumable_state` describes Python-compatible artifacts;
`resume_encryption` is an explicit operation and verifies full equality against
the remaining source/backup before publishing. Partial/ambiguous candidates are
refused without deleting any copy. This adds a verified recovery operation;
Python's `secrets init-resume` only reports paths for manual recovery.

`encryption::create_empty_database` prepares a private Python-named encrypted
sibling, closes and verifies it on a fresh connection, syncs the file and
directory, then publishes it with the existing no-overwrite rename. The secrets
initializer holds maintenance ownership; the helper does not acquire leases.
Existing candidates and populated live files are
refused; interruption artifacts remain inspectable. `verify_encrypted` returns
SQLite/cipher integrity summaries and the `sqlite_master` row count without
creating or migrating a schema. The [partial secrets checkpoint](rust-shell.md#encryption-service-checkpoint-r5-partial)
records its killed-child matrix and remaining native/backup gates.

`encryption::change_passphrase` authenticates the current key and operator
binding before invoking the caller's required `before_rekey` callback. That
callback must arm the existing desktop/operator native-credential stale guards;
the storage layer never clears them on error or contacts a keyring. Rekey is
verified through a fresh keyed connection and integrity checks. Runtime callers
must close their own connections before maintenance, just as Python's command
does. The APIs are not yet wired to a Rust CLI. Tests compare Python and Rust
encryption/rekey in both directions, preserve the rollback content, and kill
real children at eleven export/publication checkpoints before recovery.

```sh
mise exec rust@1.99.1 -- cargo build -p kassiber-store --example kassiber-storage-probe --features test-hooks --locked --offline
uv run --locked python -m pytest tests/test_rust_store_migrations.py -q
```

```sh
mise exec rust@1.99.1 -- cargo test -p kassiber-paths -p kassiber-ownership -p kassiber-store -p kassiber-secrets --locked --offline
mise exec rust@1.99.1 -- cargo clippy -p kassiber-paths -p kassiber-ownership -p kassiber-store -p kassiber-secrets --all-targets --locked --offline -- -D warnings
mise exec rust@1.99.1 -- cargo fmt --all -- --check
uv run --locked python -m pytest tests/test_rust_store_interop.py -q
uv run --locked python crates/store/tools/export_schema.py --check
```

Fetch dependencies during setup; tests build the example offline. All books,
owner namespaces and HOME/XDG inputs are disposable. The native Linux test
creates only a UUID-namespaced synthetic service/account, verifies an in-place
Python→Rust→Python update, and deletes it; absent/locked Secret Service skips
without prompting or enumerating unrelated items. The module belongs to
`serial-integration` in `scripts/python_test_shards.py`.

On a Linux host with GNOME Keyring and `dbus-run-session`, run
`uv run --locked python crates/store/tools/test_private_keyring.py` to execute
that native test on a private bus with a disposable keyring. This gives positive
interop evidence even when the caller's own Secret Service is unavailable.

### R3b legacy project migration

`projects::ensure_default` ports `projects.ensure_default_project`
and deferred workspace validation after unlock. It preserves Python's single
container for multiple workspaces and reuses the existing file publication and
catalog helpers. The standalone Python fix and Rust both resume pending legacy
artifact moves from an atomic target marker before catalog selection, including
when the legacy database has already moved. Root cutover remains a separate
preceding operation. The binary's explicit bootstrap enables this API;
normal resolution and admission-only `Book::open` do not enable migration.

R3b acceptance compares complete normalized database dumps and byte-exact non-database files
with modes. SQLite counters and SQLCipher ciphertext are outside logical-dump
equality. Explicit network partition export belongs to R5 exports.
