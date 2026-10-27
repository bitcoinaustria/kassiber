# R3 storage implementation status

## Main-stack integration

`t3code/rust-backend-r3` stacks all 40 storage commits from
`ce382b13..0970f649` onto the R2 tip `3265d832`. The cherry-picked storage tip
is `d662807c`. Workspace members and license rows retain both branches;
offline Cargo resolution retains every original registry version and checksum
(44 main-stack pins and 168 storage pins), with no new registry versions.
Custody keeps SHA-256 0.10 and storage keeps 0.11. `paths` remains the sole root
resolver; `runtime` contains shell contracts and status projections.

The real binary now selects `Book::open_migrating` for supported book creation,
unlock and the ordered compatibility pass. `init`, `status` and daemon
ready/status/unlock/lock/shutdown use real storage. Descriptor, bound remembered
unlock and TTY input use the storage unlock session; the binary explicitly
enables the pinned native credential adapter. The default hidden-home cutover
and legacy flat-project migration run at explicit command bootstrap. Existing
journal modes are preserved or refused before writes. Authored consolidation
and componentless matching remain `unsupported_schema_layout`, with a hint to
migrate using Python first. **This is supported runtime integration, not full
R3 acceptance.**

The daemon verifies its reserved file identity before opening/migrating,
retains compatible Python role reservations and binds an inode
created by its first open without releasing its path reservations. Failed
binding closes the connection before release. Legacy copying refuses existing
held local owner reservations and nonempty source journals. That does not
complete the concurrent-copy/crash matrix. No storage command contacts a
configured backend. The production build has no fake store; its explicit
test-harness fake remains for deterministic supervisor protocol and
close/release-failure injection only. Python shell parity now uses real books.

Cross-language binary tests cover init/status in both directions, selected
context and stored defaults, dotenv seeding and ephemeral process overrides,
SQLCipher fd unlock and journal preservation, daemon ownership exclusion and
release in both directions, new-book daemon lock retention, encrypted daemon
authentication/lock/shutdown, reverse secret-store control, hidden-home and
legacy-project migration, held-owner/pending-journal refusal, and new-default
creation with unported commands still failing before bootstrap. Both pair-free
historical fixtures compare complete logical dumps with Python after the real
binary opens them; both originals retain the typed consolidation refusal.

The combined workspace exposed an admission regression: serde_json's
`arbitrary_precision` feature allowed overflowing exponent literals through the
conservative admission parser. The explicit finite-number guard restores
`unsupported_schema_layout` and is covered by Rust and Python regressions.

Integration verification on Linux uses Rust 1.99.1, locked uv/Python 3.11,
`MATURIN_NO_INSTALL_RUST=1`, offline dependency caches, disposable HOME/XDG/data
roots and isolated ownership namespaces. `uv sync --locked --python 3.11`
succeeded. No account catalog, real book or real keyring was used.

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | Pass |
| Workspace clippy, all targets, locked, `-D warnings` | Pass |
| Binary clippy with `test-harness`, all targets, locked, `-D warnings` | Pass |
| `cargo test --workspace --locked` | Four `kassiber-net` loopback binds refused by the sandbox (`Operation not permitted`); eight net tests pass. No escalation |
| Workspace tests excluding `kassiber-net` | 309 passed; no ignored tests |
| `cargo test -p kassiber --features test-harness --locked` | 14 passed |
| Required combined shell/CLI-runner/inventory/shard parity suite | 67 passed, 86 subtests passed; no skips |
| Store interop and migration modules | 361 passed, three native keyring/platform skips |
| Full `serial-integration --ci` shard | 733 passed, 18 live/native skips; no deselections or xfails |
| Classifier and contract checks | Pass; generated files regenerated with their official update commands |
| Custody mutation check | All 10 killed; unmutated baseline passes |
| Registry pins, checksums and resolved dependency edges | Every originating branch entry preserved; no upgrades or new registry versions |
| Interactive TTY and real remembered-keyring unlock | Not run; descriptor and injected credential-session tests do not establish these native/manual gates |

The temporary local venv ownership-isolation hook was removed after verification.
The earlier branch reports below remain historical evidence; their private-only
runtime status is superseded by this section. Native platform, native-keyring
and remaining durability gates remain open. The full repository quality gate
and release packaging were not run; this is a local implementation branch,
with no push or PR.

## Original storage-chain evidence

R3b continuation: `t3code/r3b-partitions-2`, based on `0df35af0` from
`t3code/r3b-partitions`. The [original stop report](rust-partition-migration-stop.md)
records the earlier investigation; the two contract questions are now resolved.
**R3 remains incomplete.** Legacy project migration is private behind
`test-hooks`; `Book::open` and read-only runtime resolution retain their existing
admission and Python handoff barriers.

## R3b legacy project migration

Database equality means a **complete normalized logical dump**: every schema
object, internal table, typed row, SQLite storage class and exact float hex.
Physical page bytes are excluded: a writing Python open legitimately changes
SQLite's change counter and SQLCipher ciphertext. Every non-database file stays
byte-exact, including modes; paths and injected times are compared verbatim.
The former two physical-byte strict xfails are now logical-dump assertions.
No xfails remain in the project acceptance module.

Standalone Python fix `d2505007` changes only `kassiber/projects.py` and its
regressions in `tests/test_projects.py`, so it can be taken to `main` separately.
It uses **marker-driven resume**, with the database moved last. Reordering alone
was insufficient: `_register_existing_default_project_after_legacy_recovery`
never finished the marker, and a crash after the last move left no discoverable
source database. The target marker now atomically records all pending source
paths and one backup destination before any move. Startup finishes that intent
before the populated-catalog fast path, accepts a source already renamed to its
recorded destination, then publishes the complete marker. The same backup is
reused across restarts. A second timestamped backup would be acceptable if the
only difference were its placement, but these tests need no such exception and
compare the whole tree exactly. Conflicting existing destinations are preserved.

`crates/store/src/projects.rs` ports discovery, plaintext-only workspace counting,
stale-stage replacement, artifact copying, reports, promotion, cleanup, catalog
selection/metadata refresh and deferred SQLCipher workspace validation. It reuses
the existing private atomic writer, exclusive rename and catalog publisher;
root cutover remains the existing separate, preceding operation. There is no
new runtime crate, automatic workspace split, journal-mode transition or product
command. The copied policy inventory is pinned against Python by a drift test.
The R1 conservative catalog/metadata admission boundary remains in force.

| Starting layout | Plaintext, one/two workspaces | SQLCipher, one/two workspaces |
| --- | --- | --- |
| Flat `state/data/kassiber.sqlite3` | Python reference and Rust parity pass | Python reference and Rust parity pass |
| Flat Kassiber XDG | Pass | Pass |
| Flat Satbooks / `satbooks.sqlite3` | Pass | Pass |
| Stale `default.migrating` | Pass | Pass |
| Promoted target plus legacy source | Pass | Pass |

All **20 Python reference and 20 Rust parity cases** retain one default project,
all workspaces, both database dumps, catalog selection and the complete tree.
Both runtimes reopen the result unchanged at the stated logical boundary.
Additional acceptance covers explicit-state-root bypass, populated catalogs,
backup-name collisions, missing optional artifacts and Python's malformed
marker, invalid-intent, missing/wrong-key and key-on-plaintext error codes and
retryability.

The standalone regression kills real Python children before and after each of
the five moves and after intent/completion publication, for both codecs:
**24 cases**. Differential acceptance kills both runtimes at **23 boundaries**:
stage creation, five artifact copies, initial marker, report, before/after
promotion, intent, before/after every move, completion and catalog publication.
Both codecs and all four starter/restart combinations give **184 cases**.
Two deferred SQLCipher validation boundaries add **eight cases**; the other
four layouts add **16 cross-runtime final-move cases**. No old artifact,
incomplete marker or catalog discrepancy is accepted; all retained and promoted
databases receive integrity, foreign-key and complete-dump checks.

`book_network_migration.py` is an explicit user-triggered encrypted export,
not automatic layout migration. Its Rust port belongs to **R5 exports**, as do
its archive and export-publication acceptance; it is not an unfinished R3b
partition operation.

Current R3b verification on Linux:

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | Pass |
| Locked/offline workspace clippy, all targets/features, `-D warnings` | Pass |
| Full locked/offline workspace tests | Eight `kassiber-net` tests pass; four fail because the sandbox denies loopback binds (`Operation not permitted`). No socket escalation |
| Workspace tests excluding `kassiber-net` | 237 passed |
| Complete project acceptance module, including Python reference and Rust parity | 264 passed in the full serial shard; no xfails |
| Store migration + interop modules and `tests/test_projects.py` | 410 passed, two native-platform skips, one real Secret Service test explicitly deselected |
| `tests/test_projects.py` separately | 49 passed, including 24 standalone killed-child regressions |
| `scripts/python_test_shards.py run serial-integration --ci` | 733 passed, 17 skipped, one real Secret Service test explicitly deselected |
| Shard inventory validation and `tests/test_ci_shards.py` | Pass; 15 tests and 40 subtests passed |

All runs use `mise exec rust@1.99.1 uv@latest`,
`MATURIN_NO_INSTALL_RUST=1` and `uv sync --locked --python 3.11`.
Setup succeeded offline from the disposable dependency cache after correcting
mise/rustup/uv lookup under disposable HOME. HOME/XDG/data roots and ownership
namespaces are disposable. A temporary local venv `.pth` hook isolates child
Python ownership paths while preserving the repository's egress guard; it was
removed after verification. No real book, catalog or native keyring was used.
The live/native skips are unverified gates. Manual second-pass review and
staged/unstaged `git diff --check` pass; no push or PR was made.

Ordered-pass continuation: `t3code/r3a-ordered-migration-2`, based on
`a13704bb` from `t3code/r3a-ordered-migration`. Python fix `d8216f21` was
cherry-picked as `a3885086`; Rust follows its corrected view-creation order.
The first ordered-pass branch was based on `4a9c002c` from
`t3code/r3-storage-2`. Prior continuation history below refers to those branches.

Continuation branch: `t3code/r3-storage-2`, based on `af7a1b3a` from
`t3code/r3-storage` (R1 base `9d49cd7b`).
Python's migration ordering changes only through that cherry-pick; its DDL and
historical fixture bytes are unchanged. Commits are local; nothing was pushed
and no PR was opened. **R3 is incomplete and must not be accepted or enabled as
a complete runtime migration.** The compatibility helpers remain private behind
Rust tests / `test-hooks`; `Book::open` retains read-only admission.

## Ordered-pass attempt

The private executable `compat_ordered::migrate` now follows Python `open_db`:
base/preflight schema, ordered `ensure_schema_compat`, database identity, then
graph installation. It reuses the existing ownership, Liquid, swap, durable
anchor/evidence and schema helpers, and splits the four initial semantic
markers from the later valuation marker. The opt-in probe's `migrate` operation
supports plaintext and SQLCipher; it refuses any needed journal-mode change
before writes. No runtime command or `Book::open` enables migration.

The acceptance was written and run first: all eight original/variant and
plaintext/SQLCipher cases failed on the absent Rust operation. After wiring,
both original fixtures reach **`refresh_legacy_authored_components`** and return
`unsupported_schema_layout` (`reason=legacy_authored_custody`). Their complete
dumps match Python at entry to that step, including audit bytes, internal
objects, storage classes and exact floats; Python can finish migrating them.
Authored pair/payout consolidation, revisions/issues and matcher-based
componentless dismissal recovery are deferred until after R2's matcher port.
Affected books must use Python in the meantime. Neither fixture reaches the
componentless matcher boundary first.

**The historical custody-view blocker is resolved; all eight ordered fixture
acceptance cases pass.**
Each fixture variant removes only its single `transaction_pairs` row before
either implementation runs. Observations, concurrent authored components,
wallet metadata and retained journal rows remain. Rust then completes and its
entire dump equals Python's complete first-pass dump, for both codecs. Python
reopen changes nothing, and a repeated Rust pass followed by Python reopen
also changes nothing. Python fix `d8216f21` moves
`_ensure_custody_projection_relation_view` directly after
`_ensure_custody_leg_role_schema`; Rust now uses that same order. This prevents
SQLite's rename handling from leaving the view pointed at dropped
`__pre_suspense` tables. `HistoricalUpgradeSessionTests` separately checks that
the original fixtures' view is readable after one Python open and that the
next open changes no schema object. No values are masked and no extra reference
pass is added. The creation transcript only moves the two view statements;
the compatibility manifest only updates the reference function's source hash.
No additional first-pass parity or idempotency blocker was found in these cases.
This accepts the tested private fixture paths, not R3 or a production cutover.

Current continuation verification on Linux:

- `cargo fmt --all --check`: pass.
- Locked/offline workspace clippy, all targets/features with `-D warnings`:
  pass.
- Full workspace test attempt: **eight `kassiber-net` tests pass; four fail**
  because the sandbox refuses loopback binds (`Operation not permitted`). No
  socket escalation was attempted. Workspace excluding `kassiber-net`:
  **237 passed**.
- Ordered fixture acceptance: **eight passed**: four pair-free full-pass/reopen
  cases and four original-fixture exact-prefix/R2-refusal cases.
- `tests/test_rust_store_migrations.py`,
  `tests/test_historical_custody_compatibility.py` and
  `tests/test_rust_store_interop.py` together: **365 passed, two native-platform
  skips, four subtests passed, one real Secret Service test deselected**. No
  real keyring was accessed.
- `scripts/python_test_shards.py run serial-integration --ci`, with only that
  Secret Service test deselected: **469 passed, 17 skipped, 11 subtests passed,
  one deselected**. Live/native skips remain unverified gates.
- Shard inventory validation: pass; no test module was added.

Previous attempt verification (`a13704bb`, before the Python fix):

- `cargo fmt --all --check` and locked workspace clippy, all targets with
  `-D warnings`: pass.
- Full workspace test attempt: eight `kassiber-net` tests pass; four fail
  because the sandbox refuses their loopback binds (`Operation not permitted`).
  No socket escalation was attempted. Workspace excluding `kassiber-net`:
  **237 passed**.
- New fixture acceptance: **four exact-prefix/refusal cases pass**, and **four
  pair-free cases fail only on Python reopen** after exact first-pass parity.
- Combined store migration/interop modules: **357 passed, two native-platform
  skips, four known acceptance failures**. The real Secret Service test was
  explicitly deselected; no real keyring was accessed.
- Full `scripts/python_test_shards.py run serial-integration --ci` (with only
  that Secret Service test deselected): **465 passed, 17 skipped, four known
  acceptance failures, one deselected**. Live/native skips are not passes.
- Shard inventory validation: pass. Tests extend the existing module; no new
  module or disposition entry was added (`tests.rust_port.classify_tests` is
  absent on this base).

Both attempts use disposable data/HOME/XDG and ownership namespaces. A
temporary local venv `.pth` hook isolates Python children as well as the parent
and is removed after verification. Beyond the cherry-picked Python ordering
fix, no product Python code or historical fixture bytes are changed.

## Continuation verdict and stop point

This section records the previous worker's `4a9c002c` stop state. The
ordered-pass attempt above supersedes its orchestration status.

**R3 is still incomplete against plan 21 Verification 7.** This continuation
adds executable, private helper ports for inline retired ownership epochs,
Liquid asset/identity/fingerprint repair, the complete swap-schema helper,
missing/invalid database identities, and the grouped durable-anchor/evidence
header backfill with byte-exact `custody-durable-evidence-v1` audit JSON.
It does **not** add an executable complete `ensure_schema_compat` pass.
`Book::open` and its unsupported custody matcher refusal are unchanged.

The remaining custody steps cannot be represented faithfully as storage row
copies. `custody_authored_migration._activate_native_review` calls
`create_component` / `update_component` / `activate_component`; consolidation
also materializes existing components through `get_component`, captures
authored evidence, validates residuals, and appends immutable revisions/issues.
`custody_gap_reviews.backfill_legacy_componentless_review_relations` calls
`custody_gaps.load_gap_search_result(include_journal_claims=False)` and requires
both the original gap ID and the complete ranked candidate fingerprint. This
base has neither Rust implementation. Work stops at that dependency under the
owner's instruction to report rather than approximate an unreproduced step.
No custody authority shortcut or new product decision is introduced.

The new helpers retain the existing explicit limits for collection-to-string
coercion in identity fields and excessive JSON nesting. Liquid nonfinite
Decimal inputs fail closed with no writes. These are not universal parity
claims for arbitrary Python values.

### Continuation verification on Linux

- `cargo fmt --all --check`: pass.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: pass;
  the additional all-features clippy run also passed.
- Full locked/offline workspace test attempt: four `kassiber-net` tests fail
  because the sandbox refuses loopback binds (`Operation not permitted`);
  the crate's other eight tests pass. No socket escalation was attempted.
- Workspace tests excluding `kassiber-net`: **236 passed**.
- Final Python/Rust store selection through
  `scripts/python_test_shards.py run serial-integration --ci`:
  **353 passed, two native-platform skips**, 124 deselected (other shard tests
  and the real Secret Service test). The R3 module accounts for 242 tests.
- Python reference storage/migration modules: **103 passed, two subtests
  passed**: msat, custody migration acceptance, device-sync removal, legacy
  ledger, historical custody compatibility, authored custody, review scope,
  wages semantics, swap schema, initial schema and journal safety.
- Added helper checks: 32 identity/Liquid/swap cases, 26 durable-evidence
  cases including 16 real child terminations; included in the final selection.
- Shard inventory validation: pass. No new test module was introduced.
- Corrected full `serial-integration --ci` shard (real Secret Service test
  explicitly deselected): **461 passed, 17 skipped, 11 subtests passed**.
  Live regtest/native-platform skips remain explicit; this is not their pass.

The final store run uses the `serial-integration` shard runner with a
`rust_store` selection and explicitly deselects the real Secret Service test.
HOME/XDG/data and ownership namespaces are disposable. The earlier broad shard
skipped the native credential test before its synthetic-item operations;
no native-keyring acceptance is claimed. The reference bootstrap overrides the
fixed OS-account ownership root in child Python processes as well.
The first broad shard attempt failed 13 tests: daemon subprocesses selected
the repository's earlier `sitecustomize` instead of the disposable owner hook,
and shell snippets resolved an unconfigured uv shim. The sandbox blocked the
fixed-account ownership chmod. A local, uncommitted venv `.pth` owner hook
preserves the repository's child egress guard while isolating that namespace;
launching through the pinned `mise exec` fixes the shim. All 41 cases in the
four affected modules passed on recheck; the corrected full shard then passed
as recorded above. The local venv hook was removed after verification.

## Previous worker verification on Linux (`af7a1b3a`)

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
| Original `pre_432_5d232097.sqlite3.gz.b64`, plaintext + SQLCipher | Exact prefix; R2 refusal | From original decoded state, every object/row/audit matches Python up to `refresh_legacy_authored_components`; Rust refuses, Python finishes |
| Original `pre_435_16b7bdc1.sqlite3.gz.b64`, plaintext + SQLCipher | Exact prefix; R2 refusal | Same exact-prefix proof and authored-consolidation boundary |
| Pair-free `pre_432_5d232097.sqlite3.gz.b64`, plaintext + SQLCipher | Pass | Only the legacy pair removed before migration; exact full first-pass dump, unchanged Python reopen and repeated Rust pass |
| Pair-free `pre_435_16b7bdc1.sqlite3.gz.b64`, plaintext + SQLCipher | Pass | Same exact dump and no-change reopen/repeat proof with the corrected reference order |
| `MsatMigrationTests::test_legacy_transaction_rebuild_preserves_columns_added_after_real_schema` | Pass | Reference legacy starting point through Rust msat/base/graph helpers; complete dump; repeat and Python reopen unchanged |
| `MsatMigrationTests::test_legacy_derived_journal_quantities_migrate_to_msat` | Pass | Same checks; Rust units additionally rebuild each of the five REAL table shapes separately |
| Stranded legacy attachment / copied-provenance FK fixtures | Pass | Exact isolated Python helper comparison, authored links and child FK targets preserved |
| Authored pair/payout review-note fixture | Pass | Exact notes, idempotency and restored immutability trigger |
| Legacy ordinal and half-renamed review relation tables | Pass | Stable relation ids, auxiliary schema, complete rows, repeat unchanged |
| Authored migration-acceptance fixture with nullable evidence headers | Pass | Isolated author-snapshot commitment backfill, exact detail hashes; full pre-durable-anchor migration remains unfinished |
| Device-sync objects and empty/populated legacy-ledger fixtures | Pass | Exact isolated drops, audit bytes, retention triggers and idempotency |
| Synthetic semantic-marker, BIP329, hash, fee, inbox and gap-cache fixtures | Pass | Exact isolated-step objects and typed rows, including extended JSON and rowid cursors |
| Hidden-home root fixtures | Pass | Full tree bytes/types and inode preservation; normal/conflict/symlink/fixed-path/FIFO cases |
| Flat-book/project layouts, plaintext + SQLCipher | Reference only; Rust unported | 20 Python layout/workspace/codec cases preserve logical dumps and non-database tree bytes/modes on reopen; physical database bytes change |
| Retired ownership history | Pass for tested shapes | Exact private epoch JSON, scope fallback, duplicate-material handling and no-change Python reopen; original entropy injected before execution |
| Liquid policy-asset/fingerprint repair | Pass for tested shapes | Exact Decimal strings/hashes, collision preservation, identity enrichment and idempotency; no ownership provenance invented |
| Swap-schema orchestration | Pass for tested shapes | Legacy UNIQUE rebuild, ordered additive columns/indexes, stale fee cleanup and payment-hash backfill; authored consolidation remains separate |
| Missing/invalid database identity | Pass | Python UUID injection; valid IDs retained, invalid text and BLOB values replaced; Python reopens |

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
| Durable anchors/evidence audit | Exact historical pre-anchor shapes after preceding Python steps; injected errors and 16 killed-child cases on nullable-header authored snapshots across eight checkpoints, Python/Rust helper recovery | Pass for tested group shapes; full ordered migration and complete ALTER interruption matrix remain unimplemented |
| Ownership and Liquid backfills | Injected errors around row writes and commit, exact rollback/idempotency | Pass for tested errors; killed-process matrix remains open |
| Attachment rebuild | Fourteen killed-child cases, seven boundaries, either runtime resumes | Pass |
| Custody relations/evidence | Eighteen killed-child cases, either runtime resumes | Pass |
| Suspense-role rebuild | Fourteen killed-child cases, either runtime resumes | Pass |
| Root cutover | Fourteen killed-child cases; prepared proof, rename, fixed paths and catalog; either runtime resumes; EXDEV and held owner | Pass |
| Flat-book project publication | 184 publication/move cases across both codecs and all Python/Rust starter/restart combinations; eight deferred validation and 16 other-layout recovery cases | Pass for the private R3b matrix above; production admission remains unchanged |
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

Not fully ported: explicit network partition export (R5), repository resolution,
authored custody/gap/matcher service scenarios,
the complete compatibility-pass orchestration, daemon lock/unlock services, tax-platform
parsing and prior-report archives. The swap-schema helper now has isolated
orchestration parity. The R0 group assignment does not make the other scenarios
completed Rust tests.

## Remaining acceptance work / owner integration

1. Complete the ordered pass's interruption/commit matrix; existing isolated
   helper evidence does not prove those boundaries.
2. After R2's matcher port, port authored legacy pair/payout consolidation,
   revision/issue behavior and exact matcher-based componentless dismissal
   recovery, preserving custody authority barriers. R1 review finding 3
   remains incomplete for this matcher:
   books needing native candidate matching explicitly return
   `unsupported_schema_layout`; the guard no longer falsely promises that a
   Python migration can fix its own inability to inspect those candidates.
3. Compare both historical fixtures from their original decoded state through
   the full Rust pass and Python pass after the R2 dependency is available.
   The original prefix proofs and accepted pair-free full-pass/reopen checks
   above do not establish full original-book migration.
4. Complete the remaining R0 store dispositions. Legacy flat-book project
   migration now has the private R3b acceptance above; keep its production
   integration behind the complete R3 admission gate. Python keeps multiple
   workspaces in one container. Explicit network partition export belongs to R5.
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

Cross-service work remains with its owning stages: R4 owns journal snapshot
loading, ownership-index derivation and pre-build repairs; R5 owns custody
review/service commands, tax-platform parsing, prior-report archives and
backup/archive and explicit network-partition export workflows; shell/daemon owners wire unlock lifecycle,
native-store feature selection and stale-credential callbacks. R3 still owns
the remaining storage compatibility orchestration, production integration of
legacy flat-book/project layout migration and their durability. The explicit
network export gate is assigned to R5; the remaining R3 gates stay open.

Stash `876e53f0` was inspected read-only using `git stash show -p` during this
continuation, including `--include-untracked` and the third parent's
`encryption.rs`. The latter is the earlier implementation superseded by
`90a17a33`, which adds operator binding, zeroizing-literal capacity,
non-replacing publication and native-guard regression coverage. The tracked
error variants, module/probe hooks, documentation and initial tests are also
superseded. No unique work was needed; nothing was applied, popped or dropped.
The stash entry remains in the shared stack.

No accounting, tax or custody-policy decision is proposed by this branch.
The integration gates above need coordinating owners; incomplete migrations
must remain unavailable to production callers.

## Local commits

Continuation:

- `c69bfde6` (item 1, partial) Port private ownership and Liquid storage backfills
- `abd235b0` (items 1 and 5, partial) Group durable custody evidence backfills with their exact audit

Original worker:

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
