# Rust existing-book runtime (R1)

Python remains the reference and initializer. R1 adds four library crates;
none is a second accounting implementation. The test-only Cargo example
`kassiber-store-probe` is not installed or included in release artifacts.
No Python product behavior changes in this slice.

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
  `OwnerNamespace::isolated` is explicit test/embedding injection, never an
  environment-selectable product namespace.
- `kassiber_store::Book::open(data_root, OpenOptions)` opens an existing book.
  Options include `passphrase`, `expected_database_identity`, and
  `expected_project`. The book exposes `instance_id`, `encrypted`, `database`,
  `schema_fingerprint`, and its trusted `rusqlite::Connection`. Pass the
  lease's project in every long-lived open, including locked-state retries.
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
  secrets re-export them. Map `error.code`, `message`, `details`, and
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
R1 exposes the role leases but does not implement administrative commands.

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
typed `not_initialized`. A legacy default
root that would require cutover, a project-layout migration, or a conflicting
migration proof yields `requires_python_migration`. Explicit data-root
selection remains available without default migration. Callers must hand
initialization and migration back to Python, not retry as a fresh Rust book.

Catalog and settings readers accept the Python schema-1 object shape and
normalize valid project ids/names like Python. The deliberate R1 conservative
boundary refuses malformed metadata, unknown schema versions, symlinked files
or their immediate config directory, nonregular files, another owner, and
POSIX group/world permissions. Missing settings are empty authority, never
remembered-unlock permission. Python's general loaders sometimes silently
return empty data or normalize older/unknown metadata; Rust refuses those
inputs instead of guessing or replacing them. It also bounds metadata at
8 MiB. These refusals require recovery through Python; they do not change its
loaders. Saved manifest paths do not override computed routing.

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
around the gate when `expected_project` is supplied. No migration, generated
identity, or schema DDL runs on a runtime open.

The checked-in `crates/store/fixtures/current-schema.json` freezes every
non-internal `sqlite_master` table, index, trigger and view with its full SQL
(including column types, defaults, constraints, foreign keys and partial
indexes), after Python's current `open_db`, including graph-index installation.
The gate compares the complete sorted objects, validates all five mandatory
migration-audit id/name/version triples, and accepts only the two known
additional historical audit names. Impact payloads and timestamps are book
specific and are not compared with a fresh book's values. A read transaction
keeps the gate's schema and semantic reads consistent. The public SHA-256
schema fingerprint hashes the compact JSON object list; the audit gate is
validated separately.

`semantic-probes.sql` and the durable-review relation checks refuse pending
source-link/target snapshots, provider names, ownership-history cleanup,
wallet-scoped labels, legacy custody consolidation, anchors/evidence,
payment hashes, stale same-asset fees, missing graph clocks, and gap-review
backfills. R1 conservatively refuses some potentially compatible rows instead
of porting migrations: raw 64-character asset identifiers, incomplete LBTC
observations, payment-hash-shaped raw input, pending authored migration issues,
and ambiguous legacy gap recovery. Schema histories with retained legacy
columns, populated removed ledgers, or equivalent but different DDL also fail
the exact-schema gate. R3 must broaden acceptance only with parity evidence;
R1 is not a claim to open every historically migrated layout.

After admission, `busy_timeout=30000`, `temp_store=MEMORY`,
`cache_size=-16000`, and `foreign_keys=ON` match Python. Encrypted connections
explicitly disable mmap; plaintext requests 268435456 bytes. Journal selection
queries **this connection's** `sqlite_version()`: WAL/NORMAL for 3.51.3+, the
3.50.7+ backport, or the 3.44.6+ backport, otherwise DELETE/FULL. Unknown
strings fail closed. A refused mode is an error, never a silent fallback.

`Book::hot_backup(&mut destination_book)` uses SQLite's incremental online
backup API with a bounded retry deadline, preserves the source database ID,
and refreshes the destination's cached ID. Both books must already exist,
have the same encryption mode, and name distinct canonical files. This API
cannot implicitly decrypt a source. It does not publish a backup archive,
rebind destination unlock settings, create an artifact, or authorize restore.

R3 must implement creation, exact compatibility and semantic migrations,
legacy state/project migration, rekey, atomic backup/restore publication,
permissions/durability of new artifacts, crash/failure injection, recovery,
and the full storage durability gates in plan 21. Current tests establish
R1 interop, not crash-safe migrations or production cutover readiness.

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
No status/availability call contacts a store. Desktop AI and biometric
namespaces are separate and never used for CLI remembered unlock.

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
The two codecs may embed different SQLite releases; their own connection
versions select journal mode. Build-time crypto overrides need explicit
packaging verification. Native macOS/Windows round trips, Windows share-mode
and SID/file-id parity, macOS ACL/search-list behavior, signed executable
identity, packaged crypto linkage, and signed Touch ID hardware validation
remain platform acceptance gates. Non-default/alternate Secret Service
attribute schemes are not inferred; ambiguous stores fail closed.

## Verification

### R3 creation work

`Book::create` is an explicit empty-book operation, separate from R1's
existing-book admission. It refuses nonempty schemas and symlink paths, creates
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
The optional `test-hooks` Cargo feature enables only the development probe.

Storage-only Rust ports currently cover initial-schema transaction/rollback,
all per-statement creation failure boundaries, the private catalog writer,
and atomic settings replacement. The R0 `store` disposition also includes
journal orchestration, network partitioning and prior-tax-report services;
those require their owning service stages. Historical compatibility, semantic
migrations, root migration and encryption lifecycle remain R3 acceptance work.

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
