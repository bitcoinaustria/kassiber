# R3 storage implementation status

Branch: `t3code/r3-storage`, based on R1 `9d49cd7b`.
Python product code and DDL are unchanged. Commits are local; nothing was pushed
and no PR was opened. **R3 is incomplete and must not be accepted or enabled as
a complete runtime migration.** The compatibility helpers remain private behind
Rust tests / `test-hooks`; `Book::open` retains read-only admission.

## Verification on Linux

- Rust 1.99.1: workspace formatting and workspace clippy, all targets/features,
  `-D warnings`, locked/offline: pass.
- Workspace tests with `kassiber-net` excluded: **235 passed**.
- Full workspace attempt: four unchanged `kassiber-net` tests cannot bind their
  owned loopback sockets in this sandbox (`Operation not permitted`). The eight
  other tests in that crate passed in the original attempt. No socket permission
  escalation was attempted.
- Final combined Python/Rust store modules: **289 passed**, two native-platform
  skips, one real-keyring test deselected. Dumps include every internal SQLite
  object and storage class. The R3 module also passed independently: 178 tests.
- Python reference storage/migration checks: **246 passed, four subtests passed**
  (203 in the main run and 43 in a separate supplemental run).
  Modules: msat, custody migration acceptance, device-sync removal, legacy
  ledger, historical custody compatibility, authored custody migration, review
  scope migration, wages semantics, swap schema, initial schema, journal safety,
  data roots, projects and secrets smoke. Supplemental modules: managed settings,
  repository resolution, import batches, BTCPay commercial provenance and book
  network migration.
- Shard inventory validation: pass; the new module is in `serial-integration`.
- Native Secret Service acceptance was attempted only on a disposable private
  bus/keyring. The sandbox refused the temporary D-Bus socket; it did not run.
  No real keyring was read or written.

The Python reference run used a temporary `sitecustomize` to redirect Python's
fixed OS-account ownership namespace as well as disposable HOME/XDG roots.
Tests never used the account's actual books, ownership files or credential
store. Setup fetched packages; all Cargo test/build calls were locked/offline.

## Fixture parity

| Starting point | Exact normalized result | Scope / remaining gate |
| --- | --- | --- |
| Empty plaintext book | Pass | All objects, internal objects, typed rows, audits; Python reopen changes nothing |
| Empty SQLCipher book | Pass | Same checks; Python opens Rust output and Rust opens Python output |
| `pre_432_5d232097.sqlite3.gz.b64` | Partial | Python-migrated book accepted by Rust; isolated suspense-role rebuild matches Python after Python runs preceding steps; full Rust migration **not implemented** |
| `pre_435_16b7bdc1.sqlite3.gz.b64` | Partial | Same scope and remaining full-migration gate |
| `MsatMigrationTests::test_legacy_transaction_rebuild_preserves_columns_added_after_real_schema` | Pass | Reference legacy starting point through Rust msat/base/graph helpers; complete dump; repeat and Python reopen unchanged |
| `MsatMigrationTests::test_legacy_derived_journal_quantities_migrate_to_msat` | Pass | Same checks; Rust units additionally rebuild each of the five REAL table shapes separately |
| Stranded legacy attachment / copied-provenance FK fixtures | Pass | Exact isolated Python helper comparison, authored links and child FK targets preserved |
| Authored pair/payout review-note fixture | Pass | Exact notes, idempotency and restored immutability trigger |
| Legacy ordinal and half-renamed review relation tables | Pass | Stable relation ids, auxiliary schema, complete rows, repeat unchanged |
| Authored migration-acceptance fixture with nullable evidence headers | Pass | Isolated author-snapshot commitment backfill, exact detail hashes; full pre-durable-anchor migration remains unfinished |
| Device-sync objects and empty/populated legacy-ledger fixtures | Pass | Exact isolated drops, audit bytes, retention triggers and idempotency |
| Synthetic semantic-marker, BIP329, hash, fee, inbox and gap-cache fixtures | Pass | Exact isolated-step objects and typed rows, including extended JSON and rowid cursors |
| Hidden-home root fixtures | Pass | Full tree bytes/types and inode preservation; normal/conflict/symlink/fixed-path/FIFO cases |

**No values are masked.** The normalized dump includes every `sqlite_master`
object, all tables including internal tables, PK-ordered rows, SQLite storage
classes and exact float hex. Fresh-book `database_instance_id` and SQLite audit
`strftime` are injected into both implementations before execution. Existing
fixture entropy is generated once and copied identically to each starting root.
Cipher page salt/layout bytes are not a normalized logical database dump.

## Durability matrix

| Gate | Evidence | Result |
| --- | --- | --- |
| Fresh schema atomicity | Before/after every creation statement and commit; eight killed-child cases resumed by Python/Rust | Pass |
| Private file publication | Owner-only files/directories, symlink refusal, atomic replacement, directory fsync; old-or-new bytes on injected errors | Pass for implemented file operations |
| Exclusive publication | Existing destination never replaced; no cross-device copy/delete fallback | Pass |
| Audit behavior | Five semantic migrations plus device-sync removal; exact impact JSON, idempotency, immutable audit guard and rollback | Pass for these helpers |
| Custody schema/terms | Counts/FKs, scope and immutability objects, review-note trigger behavior, failed suspense rebuild rollback | Pass for isolated helpers |
| Attachment rebuild | Fourteen killed-child cases, seven boundaries, either runtime resumes | Pass |
| Custody relations/evidence | Eighteen killed-child cases, either runtime resumes | Pass |
| Suspense-role rebuild | Fourteen killed-child cases, either runtime resumes | Pass |
| Root cutover | Fourteen killed-child cases; prepared proof, rename, fixed paths and catalog; either runtime resumes; EXDEV and held owner | Pass |
| Plaintext export / rekey interop | Both languages, `auto_vacuum` 0/1/2; user_version, integrity, FK checks and retained rollback copy | Pass |
| Encryption publication | Eleven killed-child export/publication boundaries plus injected errors; verified Rust resume and Python reopen | Pass for selected boundaries; partial export and full rekey kill matrix still pending |
| Rekey credential guard | Wrong key never arms guard; authenticated guard precedes rekey; post-rekey ambiguity leaves it armed | Pass; CLI/native guard integration pending |
| Backup API | Python backup/restoration of Rust-written plaintext and SQLCipher books, then Rust opens; Rust backup to exact integer in-memory clone | Pass for these directions; full encrypted Rust backup/failure matrix pending |
| Held connection journals | Python connection held across Rust write and reverse direction; encrypted DELETE/FULL and existing plaintext policy | Pass |
| Concurrent writers | Three Rust processes plus Python, 80 committed increments, plaintext and SQLCipher; busy/retry and integrity/FKs | Pass |
| Journal safety / memory clone | Embedded engine version rules, no WAL promotion during coexistence, in-memory backup clone retains MEMORY/FULL | Pass |
| Linux native keyring | Disposable private bus only | Blocked by sandbox socket binding |
| Windows/macOS | Native handles/path identity/case/sign/realpath and cipher round trips | Not run; explicit native gates |

This is not the complete plan Verification 7 matrix: every physical durable
call, every migration starting point and every interruption path still need
coverage before R3 acceptance. Python's encryption resume command reports
manual-recovery paths; automatic verified publication recovery is Rust-only.

## R0 store dispositions carried into Rust tests

The R0 inventory was read from `t3code/rust-backend-r0`; it contains broader
service tests as well as storage. Completed storage intent ports include:

- `test_database_initial_schema.py`: transactional fresh initialization and
  rollback of all schema objects (all statement boundaries).
- `test_database_journal_safety.py`: known fixed branches, safe rollback/WAL
  selection, refusal behavior and the in-memory accounting clone. Coexistence
  rules deliberately supersede automatic Rust WAL promotion.
- `test_msat_migration.py::MsatMigrationTests`: all five quantity rebuild shapes,
  preservation of later transaction columns and retry without double scaling.
- `test_wages_semantic_migration.py`: wages, pool-basis, custody-hold and engine
  invalidation/marker intent, including fresh-marker no-op behavior; Austrian
  valuation support is additionally pinned.
- `test_custody_authored_migration.py`: review-note backfill/refreezing and
  suspense-role rebuild preserving economic-term FKs. The other authored
  consolidation tests are not ported.
- `test_review_regressions.py::ReviewRegressionTest::test_migration_collapses_wallet_scoped_bip329_duplicates`:
  latest fields and merged JSON, with rollback and idempotency.
- Storage portions of `test_data_roots.py`: native/relative-XDG paths,
  hidden-home meaningful state, conflicting roots, inode-preserving atomic
  rename, interrupted completion, EXDEV, fixed paths, symlinks, special files,
  active owners and catalog rebasing. Native Windows cases remain gated.
- Storage portions of `test_managed_settings.py::ManagedSettingsTests` and
  `test_projects.py::ProjectCatalogTests`: private atomic settings/catalog
  publication. Full classes and service workflows are not claimed complete.
- `test_projects.py::DaemonProjectSwitchTests` close-before-release and failed
  close retention intent in `failed_close_retains_owner_until_connection_can_close`;
  daemon command integration belongs to its owner.

Not fully ported: network partitioning, repository resolution, legacy project
splitting, all custody schema/gap/matcher service scenarios, complete swap-schema
orchestration, daemon lock/unlock services, tax-platform parsing and prior-report
archives. The R0 group assignment does not make these completed Rust tests.

## Remaining acceptance work / owner integration

1. Implement the complete ordered `ensure_schema_compat` pass. The generated
   95 main-column definitions and helper SQL are data, not an executable port.
   Port inline ownership epochs, Liquid asset/fingerprint backfills, complete
   swap orchestration, durable-anchor/commitment grouping and the exact
   `custody-durable-evidence-v1` audit, and missing database identities.
2. Port authored legacy pair/payout consolidation, revision/issue behavior and
   exact matcher-based componentless dismissal recovery, preserving custody
   authority barriers. R1 review finding 3 remains incomplete for this matcher:
   books needing native candidate matching explicitly return
   `unsupported_schema_layout`; the guard no longer falsely promises that a
   Python migration can fix its own inability to inspect those candidates.
3. Compare both historical fixtures from their original decoded state through
   the full Rust pass and Python pass. Require exact dumps and a no-change
   Python reopen. Isolated step parity above does not satisfy this gate.
4. Port legacy flat-book to project/network partition migrations and remaining
   R0 store dispositions. Keep cross-service behavior with its owning stage.
5. Complete the untested crash/backup/native-platform matrix. The local JSON
   decoder refuses nesting beyond 1000 and inherits pyjson encoding limits;
   runtime admission still conservatively refuses Python's extended JSON when inspected, integers beyond signed/unsigned 64-bit range and collection string coercions in identity fields.
6. CLI/daemon owners must enable `kassiber-secrets/native-keyring` where needed,
   pass the shipped plaintext Python SQLite version to new-book policy, bind
   operator children before keying, and arm native credential stale guards via
   the required rekey callback. None is wired into a Rust product command here.
7. Windows: inherited OS-handle passphrase pipes, case-insensitive path aliases,
   and Python <=3.11 file identity; macOS: signed dev_t and realpath case
   normalization; both: native crypto/keyring packaging and interop acceptance.

The earlier R3 encryption-work stash is preserved. Optional stash deletion was
rejected by automatic approval review because it could contain unique work;
no stash was deleted.

No accounting, tax or custody-policy decision is proposed by this branch.
The integration gates above need coordinating owners; incomplete migrations
must remain unavailable to production callers.

## Local commits

- `bef42221` Create Rust books with the Python schema and durable layout
- `2d02330b` Port one-shot projection semantic migrations
- `0a2e0c8e` Preserve Python-safe journal modes during Rust coexistence
- `5fe5a1ce` Accept structurally compatible Python-migrated schemas
- `138f3c26` Narrow semantic admission probes to Python write predicates
- `c169bf06` Refuse unsupported Python JSON without hiding migration inputs
- `f5b1264d` Fall through when remembered unlock enrollment is unusable
- `fa80fa90` Bind operator children to the canonical project before keying
- `04998266` Preserve actionable hints in shared storage errors
- `d414d8c2` Reserve key literal capacity before copying secret bytes
- `b4a8c978` Treat a null unlock mode as manual
- `ad0e0dab` Match Python home resolution for an empty HOME value
- `12d5527f` Report transient Python schema replacement gaps as retryable
- `b5201fd1` Keep isolated ownership namespaces inside their supplied root
- `31734b3b` Record native Windows and macOS storage acceptance gates
- `2133c15e` Isolate native credential adapters from core builds
- `952ded64` Select new book journaling from the shipped Python policy
- `fc821d9f` Retire device sync and empty legacy ledger storage atomically
- `5e227567` Publish storage artifacts without replacing existing paths
- `16a21fb1` Migrate legacy state roots with a resumable rename proof
- `90a17a33` Encrypt and rekey books with verified crash-safe publication
- `a66a0cba` Port legacy msat rebuilds and graph storage installation
- `ad35df0c` Port conditional storage schema repairs
- `95724b3e` Preserve authored custody relations and evidence commitments
- `4095e3de` Rebuild custody child storage without changing authored rows
- `8c5fe0a2` Match Python payment-hash whitespace admission
- `276b8f24` Port label, payment and derived-cache backfills
- `c22cc4db` Preserve in-memory accounting clones under journal guards
- `1abfb228` Refuse lossy Python values before semantic admission
- Final verification record: this document, strict all-object/type comparisons and the review-note trigger regression.
