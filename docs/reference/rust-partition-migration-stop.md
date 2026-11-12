# Legacy project and network partition acceptance stop

> **Historical.** This page records the Python-to-Rust port. Python was removed
> under [plan 22](../plan/22-python-removal.md); the `kassiber/` paths and Python
> tests it cites exist only at the `archive/python-backend` tag.

Historical stop record at `0df35af0`. The orchestrator resolved both questions
in the R3b continuation: the logical-dump boundary, a Python recovery fix and
private Rust parity.

Investigation on `t3code/r3b-partitions`, starting at `ca3a224a`. No Rust
migration was added or enabled. Python remains the reference. This records a
stop under the instruction to report an unmet exact behavior or a required
product decision before approximating it.

## Python entry points and order

The following order was recorded before any Rust implementation:

For ordinary default-root CLI bootstrap, `cli/main.py::_maybe_migrate_default_state_root`
first calls `db.py::migrate_hidden_home_state_root_if_needed` (subject to its
explicit-data-root/env-file and command exclusions). That existing rename-only
root cutover precedes project resolution. The project helpers' own
`default_state_root` selection is read-only; it does not perform this cutover.

1. `core/runtime.py::resolve_runtime_paths` calls
   `projects.py::project_for_runtime`, then `selected_project` or
   `ensure_default_project`. An explicit data root instead uses
   `project_metadata_for_data_root`; it does not authorize a layout migration.
2. `ensure_default_project` first honors a populated catalog. Otherwise
   `migrate_legacy_default_layout_if_needed` runs. Passing an explicit
   `state_root` to that migration returns `None` without migrating.
3. `_legacy_layout` searches `default_state_root()/data`, flat Kassiber XDG,
   then flat Satbooks, trying `kassiber.sqlite3` before `satbooks.sqlite3`.
   `resolve_effective_state_root` supplies each layout's owning directory.
4. If the default target database already exists,
   `_register_existing_default_project_after_legacy_recovery` moves legacy
   artifacts aside and registers that target. It does not compare the two
   databases or infer which workspace belongs to a project.
5. Otherwise `_workspace_count_for_plaintext` inspects plaintext only. Stale
   `default.migrating` is removed. The database, attachments, exports,
   `backends.env` and `settings.json` are copied into the staging project.
6. The staging marker is written, `_write_migration_report` publishes the
   report in the source state, and the marker is rewritten with its report path.
   `staging_root.replace(target_root)` publishes the project.
7. `_move_legacy_artifacts_aside` creates a timestamped backup and calls
   `_move_if_exists` in this exact order: **database, attachments, exports,
   backends.env, settings.json**. The target marker is then rewritten with
   `legacy_artifacts` and `legacy_artifacts_moved_at`.
8. `create_project` creates missing project directories and `_upsert_project`
   calls `write_catalog`, selecting the default project.
9. `bootstrap_runtime` initializes the runtime paths, opens/migrates the book
   with `db.py::open_db`, and calls
   `projects.py::validate_project_migration_after_unlock`. Encrypted migrations
   clear the deferred workspace-validation marker here; multiple workspaces
   generate a second report and still remain in one project.

**There is no automatic project-per-workspace or per-network migration.**
`WORKSPACE_SPLIT_POLICY` describes a policy inventory, not executable splitting.
Both plaintext and encrypted multi-workspace histories remain in one default
container. This agrees with `tests/test_projects.py::LegacyProjectMigrationTests`.

Repository resolution is separate: `core/repo/context.py::resolve_workspace`
and `resolve_profile`, and `core/repo/wallets.py::resolve_wallet`, try an exact
scoped ID first, then a case-insensitive label. Multiple label matches return
`validation`; missing matches return `not_found`. They do not select a winning
partition or migrate a layout.

Explicit network export follows a different order:

1. `core/book_network_migration.py::plan_network_partition` calls `_partition`
   inside `book_network.py::read_network_snapshot`.
2. `_partition_snapshot` validates the environment and regtest UUID, inventories
   network evidence, loads authored and provenance rows, computes relationship
   closure, and checks archived ledger rows, conflicting wallet history,
   declarations and foreign keys. The plan includes source and recipe digests.
3. `export_network_partition` recomputes the partition, checks `stale_context`
   **before** `book_network_review_required`, then checks destination and
   encryption inputs. It never rewrites or prunes the source.
4. `open_db` creates the staged target using the supplied `db_passphrase` before
   rows are copied; a SQLCipher source connection requires that argument before
   staging. Selected immutable IDs are copied, retired device-sync columns are
   omitted, wallet configuration is projected onto public watch policy, and
   processed state is reset.
5. `plan_book_network` / `apply_book_network` bind the target, invalidate its
   journals, and enforce the same inventory/declaration policy. Foreign keys
   are checked before commit.
6. A tar archive carries the database, manifest and hash-checked attachments.
   `backup/age_cli.py::encrypt_age_stream` applies outer age encryption. A
   flushed/fsynced sibling is hard-linked exclusively to the destination,
   then unlinked. This function does not fsync the destination directory.
   Backup import is another explicit action.

This base has `crates/paths` and `crates/store`, not `crates/runtime` or
`tests/rust_port/classify_tests.py`. Neither discrepancy authorizes a new runtime
architecture or a replacement classifier.

## Acceptance written before implementation

`tests/test_rust_store_partition_reference.py` first checks the reference itself.
Inputs, HOME/XDG roots, catalog, ownership namespace and child process are
disposable. Clock/identity are supplied before execution. Each interruption run
restores the same seed at the same absolute path, preserving source SQLCipher
bytes and filesystem modes; persisted paths and times are never masked.

The reference matrix covers flat `state/data`, flat Kassiber XDG, flat Satbooks
with its old filename, stale staging, and an already promoted target, each with
one/two workspaces and plaintext/SQLCipher: **20 cases**. All resulting databases,
including the retained original, have complete normalized schema/typed-row
dumps, integrity/FK checks and preserved workspace counts. Catalog selection,
all paths and modes, non-database bytes and logical Python reopen are checked.
Physical database-byte stability is a separate, explicit acceptance condition.

| Starting layout | Python plaintext, one/two workspaces | Python SQLCipher, one/two workspaces | Rust parity |
| --- | --- | --- | --- |
| Flat `state/data/kassiber.sqlite3` | Logical reopen/tree/modes pass | Logical reopen/tree/modes pass | Unported |
| Flat Kassiber XDG | Logical reopen/tree/modes pass | Logical reopen/tree/modes pass | Unported |
| Flat Satbooks / `satbooks.sqlite3` | Logical reopen/tree/modes pass | Logical reopen/tree/modes pass | Unported |
| Stale `default.migrating` | Logical reopen/tree/modes pass | Logical reopen/tree/modes pass | Unported |
| Already promoted target plus legacy source | Logical reopen/tree/modes pass | Logical reopen/tree/modes pass | Unported |

Here tree-byte equality excludes database pages, which are checked separately
and fail the strict physical-byte acceptance. Every database's normalized dump
is compared without removing objects, rows or values.

The two physical-byte acceptance cases and two killed-child acceptance cases
are strict expected failures, restricted to a dedicated mismatch exception.
Harness, keying, integrity or logical-data failures remain failures. Running
`--runxfail` exposes all four unmet requirements. These tests are **reference
evidence only**, not Python/Rust differential acceptance or a Rust parity claim.

## Observed conflicts requiring a decision

**Physical reopen is not byte-idempotent.** Python's second `open_db` keeps
every normalized schema object and typed row unchanged, but rewrites the active
database. Plaintext's SQLite change counter advances; SQLCipher also changes
physical ciphertext pages. All 20 layout/codec cases demonstrated this before
the physical-byte assertion was separated from logical reopen checks. Literal
file-byte invariance across a Python reopen cannot be the oracle's contract
without changing Python. Existing R3 acceptance instead compares logical
database dumps and expressly excludes cipher page-layout bytes. Decide whether
the requested tree-byte criterion includes database pages or follows that
existing logical-dump boundary; this branch does not silently relax it.

**Recovery does not finish the same filesystem work.** A real child is killed
immediately after `_move_if_exists` moves the legacy database and before
attachments move. Python `ensure_default_project` plus reopen succeeds, and
both complete database dumps equal uninterrupted migration. However,
`_legacy_layout` can no longer discover the source database. Python creates
the catalog and leaves the original attachments, exports, bootstrap settings
and unfinished target marker in place. They are absent from the timestamped
backup. The published copies remain present; no database or evidence loss was
observed. Repeated normal resolution cannot complete this artifact migration.

Exact filesystem recovery by either runtime needs a Python-readable durable
intent/recovery protocol, or a decision that this incomplete cleanup is an
accepted restart result. Merely adding Rust recovery cannot make an unchanged
Python runtime finish those paths. This branch changes neither the reference
protocol nor the requested acceptance condition.

## Stop state

No Rust entry point, fault boundary, journal transition, dependency, product
Python behavior, historical fixture or real book was changed. No Rust network
partition export, repository resolver or legacy project migration is claimed.
The full Rust-vs-Python layout matrix, network-export matrix and publication
interruption matrix remain to be implemented after resolving these contracts.
R3 remains incomplete and runtime admission stays unchanged.

## Verification on this branch

All checks use Rust 1.99.1 through `mise exec rust@1.99.1 uv@latest`,
`MATURIN_NO_INSTALL_RUST=1`, and the locked Python 3.11 environment. Setup ran
`uv sync --locked --python 3.11`; after DNS prevented the initial fetch, it
succeeded offline from a copied disposable dependency cache.

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | Pass |
| `cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings` | Pass |
| `cargo test --workspace --locked --offline` | Four `kassiber-net` failures: sandbox denies loopback binds with `Operation not permitted`; its other eight tests pass. No socket escalation |
| Workspace tests excluding `kassiber-net` | 237 passed |
| New reference acceptance module | 20 passed, four strict expected failures; `--runxfail` selection confirms four actual contract failures |
| Existing store migration/interop modules | 250 migration cases and 111 interop cases passed in the corrected shard; two native-platform skips; real Secret Service test deselected |
| Python reference migration/network/path/repository modules | 95 passed, two subtests passed |
| `scripts/python_test_shards.py run serial-integration --ci` | 489 passed, 17 skipped, four expected failures, 11 subtests passed, one real Secret Service test explicitly deselected |
| `python -m tests.rust_port.classify_tests update`, then `check` | Both attempted; unavailable because `tests.rust_port` is absent on `ca3a224a` |
| Existing shard inventory validation | Pass; new module assigned to serial-integration |
| `tests/test_ci_shards.py` | 15 passed, 40 subtests passed |

Reference selection: `test_projects.py`, `test_data_roots.py`,
`test_repo_resolution.py`, `test_book_network.py`,
`test_book_network_migration.py`, `test_book_network_boundaries.py`, and
`test_book_network_read_snapshot.py`.

The first serial-shard attempt had 110 interop fixture setup errors because
nested `mise`/rustup commands resolved an invalid shim under disposable HOME.
Only the temporary tool environment was corrected (installed tool paths,
offline mise and its data/config paths); the full shard was then rerun with
the results above. A temporary venv `.pth` hook redirected the fixed ownership
namespace in parent and child Python processes while preserving the existing
child egress guard; it was removed after verification. No real book, project
catalog or keyring was used. Live/native skips remain unverified gates.

Manual second-pass review and `git diff --check` pass. No push-ready claim or
complete R3 acceptance is made.
