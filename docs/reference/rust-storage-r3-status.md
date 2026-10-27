# R3 storage implementation status

Ordered-pass branch: `t3code/r3a-ordered-migration`, based on `4a9c002c` from
`t3code/r3-storage-2`. Prior continuation history below refers to that branch.

Continuation branch: `t3code/r3-storage-2`, based on `af7a1b3a` from
`t3code/r3-storage` (R1 base `9d49cd7b`).
Python product code and DDL are unchanged. Commits are local; nothing was pushed
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

**Acceptance remains blocked by the Python reference's own non-idempotence.**
Each fixture variant removes only its single `transaction_pairs` row before
either implementation runs. Observations, concurrent authored components,
wallet metadata and retained journal rows remain. Rust then completes and its
entire dump equals Python's complete first-pass dump, for both codecs.
However, `_ensure_custody_projection_relation_view` runs before
`_ensure_custody_leg_role_schema`. SQLite rewrites the view's references during
the latter's renames to `custody_component_legs__pre_suspense` and
`custody_component_economic_terms__pre_suspense`, which are subsequently
dropped. The next Python open recreates the view with the live table names.
Python-only first/second opens reproduce this same difference. Thus one-pass
byte-exact parity and unchanged Python reopen cannot both pass against this
reference. The four variant acceptance cases remain deliberately failing at
the reopen assertion. No values are masked, no extra reference pass is added,
and Python product code is unchanged. Resolve the reference ordering contract
before continuing; this branch is not accepted or ready for runtime use.

Verification for this attempt:

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

All data/HOME/XDG and ownership namespaces are disposable. A temporary local
venv `.pth` hook isolates Python children as well as the parent and is removed
after verification. No product Python code or fixture bytes are changed.

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
| Pair-free `pre_432_5d232097.sqlite3.gz.b64`, plaintext + SQLCipher | Exact full first-pass dump; reopen **fails** | Only the legacy pair removed before migration; Python changes the stale `journal_custody_projection_relations` view on reopen |
| Pair-free `pre_435_16b7bdc1.sqlite3.gz.b64`, plaintext + SQLCipher | Exact full first-pass dump; reopen **fails** | Same Python reference idempotency blocker; not an acceptance pass |
| `MsatMigrationTests::test_legacy_transaction_rebuild_preserves_columns_added_after_real_schema` | Pass | Reference legacy starting point through Rust msat/base/graph helpers; complete dump; repeat and Python reopen unchanged |
| `MsatMigrationTests::test_legacy_derived_journal_quantities_migrate_to_msat` | Pass | Same checks; Rust units additionally rebuild each of the five REAL table shapes separately |
| Stranded legacy attachment / copied-provenance FK fixtures | Pass | Exact isolated Python helper comparison, authored links and child FK targets preserved |
| Authored pair/payout review-note fixture | Pass | Exact notes, idempotency and restored immutability trigger |
| Legacy ordinal and half-renamed review relation tables | Pass | Stable relation ids, auxiliary schema, complete rows, repeat unchanged |
| Authored migration-acceptance fixture with nullable evidence headers | Pass | Isolated author-snapshot commitment backfill, exact detail hashes; full pre-durable-anchor migration remains unfinished |
| Device-sync objects and empty/populated legacy-ledger fixtures | Pass | Exact isolated drops, audit bytes, retention triggers and idempotency |
| Synthetic semantic-marker, BIP329, hash, fee, inbox and gap-cache fixtures | Pass | Exact isolated-step objects and typed rows, including extended JSON and rowid cursors |
| Hidden-home root fixtures | Pass | Full tree bytes/types and inode preservation; normal/conflict/symlink/fixed-path/FIFO cases |
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
splitting, authored custody/gap/matcher service scenarios, the complete
compatibility-pass orchestration, daemon lock/unlock services, tax-platform
parsing and prior-report archives. The swap-schema helper now has isolated
orchestration parity. The R0 group assignment does not make the other scenarios
completed Rust tests.

## Remaining acceptance work / owner integration

1. Resolve the Python reference's historical custody-view idempotency blocker
   above. The ordered private pass now exists and matches first-pass fixture
   results, but is unaccepted. Complete the ordered pass's interruption/commit
   matrix; existing isolated helper evidence does not prove those boundaries.
2. After R2's matcher port, port authored legacy pair/payout consolidation, revision/issue behavior and
   exact matcher-based componentless dismissal recovery, preserving custody
   authority barriers. R1 review finding 3 remains incomplete for this matcher:
   books needing native candidate matching explicitly return
   `unsupported_schema_layout`; the guard no longer falsely promises that a
   Python migration can fix its own inability to inspect those candidates.
3. Compare both historical fixtures from their original decoded state through
   the full Rust pass and Python pass. The original prefix proofs and pair-free
   first-pass parity above do not establish the required no-change Python
   reopen or full original-book migration.
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

Cross-service work remains with its owning stages: R4 owns journal snapshot
loading, ownership-index derivation and pre-build repairs; R5 owns custody
review/service commands, tax-platform parsing, prior-report archives and
backup/archive command workflows; shell/daemon owners wire unlock lifecycle,
native-store feature selection and stale-credential callbacks. R3 still owns
the unaccepted storage compatibility orchestration, legacy flat-project/network
partition cutovers and their durability; those gates have not been reassigned.

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
