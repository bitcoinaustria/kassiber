# Rust write transactions

Every Rust write transaction takes SQLite's writer lock before its first read.
`crates/store/tests/write_transactions.rs` enforces this over the source
tree. The rule came from the Python backend's fix `470334f61`
(`begin_write_savepoint`, `BEGIN IMMEDIATE`), removed with Python under
[plan 22](../plan/22-python-removal.md).

## Why

A transaction that begins deferred takes a shared lock with its first read.
Deferred openers include a bare `SAVEPOINT` outside a transaction, `BEGIN`, and
rusqlite's `transaction()`, `savepoint()` and `unchecked_transaction()`. When
another connection holds the writer lock, the transaction's first write must
upgrade that shared lock. SQLite refuses the upgrade at once with
`SQLITE_BUSY` and skips the busy handler, because waiting could deadlock. So
`busy_timeout` (30 s) never applies, and the request fails as `database_busy`.
Python round 11 proved this race for encrypted document imports. With another
connection holding the writer, 8 of 8 Rust document imports failed at once in
`crates/wallets/src/file_import/document/coordinator.rs`.

## Rule

A write transaction opens in one of these ways:

- `kassiber_store::begin_write_savepoint(conn, name)` for a named savepoint.
  It prepares `UPDATE settings SET value = value WHERE 0` while no
  transaction is open, then runs it right after `SAVEPOINT name`. The
  statement matches no row, fires no trigger and dirties no page, so commit
  writes nothing and stored bytes stay identical. Its only effect is to ask
  for the writer lock, which waits like any write. `RELEASE` of an outermost
  savepoint still commits. If the reservation is refused, the helper rolls
  back and releases the savepoint. The reservation is skipped when the
  connection already holds the writer (nested in a caller's write
  transaction), when it cannot write (read-only or `query_only`, such as an
  MCP pin, where no upgrade can follow), or when it has no `settings` table
  (partial test schemas, as Python's `63047edfe` checks).
- `kassiber_store::write_savepoint(&mut conn)`: the same, as a rusqlite
  `Savepoint`. It commits on `commit()` and rolls back when dropped.
- `kassiber_store::write_transaction(conn)`: an unchecked `BEGIN IMMEDIATE`.
  On a connection that cannot write it falls back to `BEGIN DEFERRED`.
- `BEGIN IMMEDIATE` or `TransactionBehavior::Immediate`. A function's own
  `SAVEPOINT` behind its own `BEGIN IMMEDIATE` (used when no caller
  transaction is open) already holds the writer.

Read-only snapshots keep a plain deferred opener, because they never upgrade.
Each snapshot with an explicit end calls `kassiber_store::debug_assert_no_write`
there, so debug and test builds catch a snapshot that wrote. Dispatchers that
serve both reads and writes (`kassiber_setup::execute`, the backend
configuration) reserve the writer for every command except
`kassiber_setup::reads_only` ones. A read therefore never waits behind a long
writer, such as a journal rebuild, and a misclassified write still fails the
debug assertion.

`crates/store/tests/write_transactions.rs` scans product sources under
`crates/*/src`. It skips `tests/`, `examples/`, `tests.rs`, `fixture.rs` and
`#[cfg(test)]` items. The scan fails on any deferred opener outside its named
allowlist and on allowlist entries that no longer exist. A third test pins the
scanner on each opener form. `crates/store/src/write_lock.rs` tests pin both
halves on plaintext and SQLCipher books:

- A bare savepoint that reads first is refused at once.
- Each helper waits for the held writer and then commits.
- A refused reservation leaves no transaction open.
- The reservation writes no bytes, nests as a plain savepoint, and skips
  `query_only` connections and partial schemas.

`crates/setup/tests/write_lock.rs` covers the setup dispatcher. Writes that
resolve their scope first (`profiles.create`, `accounts.create`) wait for a
held writer, and reads do not wait behind one.

The Python parity module `tests/test_rust_write_transaction_locks.py` held
the writer from a second connection for 1 s before each write and required
the binary to wait and succeed, on plaintext and SQLCipher books. It went with
Python; its recorded sessions replay the flows' outputs
(`scripts/replay-sessions.sh`) but not the concurrent writer, and its
SQLCipher sessions were dropped (plan 22). It covered these flows:

- daemon: document import, `ui.wallets.create`, metadata update, journals
  process, review apply and `ui.workspace.create`
- CLI: notes, tags, wallet create, rates, JSON import and its rollback,
  journals process, review apply, workspace create, and backup export and
  restore (SQLCipher only)

A daemon opens the book once, so its writes meet the held writer inside their
own transaction. A CLI process already waits while opening the book, because
the open takes the writer for its stored-error check. On the previous binary,
the daemon document import and `ui.wallets.create` failed `database_busy` at
once.

## Verification (2026-11-07)

Recorded at the time, with the Python tools that existed then.

- `tests/rust_port/repro_document_import_busy.py --impl rust --hold-writer 2`:
  the previous binary failed 8 of 8 copies with `database_busy`. This branch
  passed 200 of 200 copies with a held writer and 96 of 96 without one.
- `KASSIBER_WRITE_LOCK_COPIES=200` on the pytest module above: 1200 of 1200
  passed (6 cases × 200).
- New mutation entries, all killed by their named tests: 9 in
  `crates/store/scripts/mutations.json` and 2 in
  `crates/setup/scripts/mutations.json`.
- Stored bytes: the reservation's unit test compares the database file before
  and after. `tests/test_rust_store_*.py` passed (701, 2 skipped). The
  `tests/test_rust_journal_*.py` parity modules failed the same 29 tests on
  this branch and on the previous binary, so nothing changed. The failures
  are listed in the branch handoff.

## Inventory

Classification: **write** (reserves the writer first), **read** (read-only
snapshot, stays deferred), **open** (one-time open migration or book creation,
which runs inside `Book::open_migrating`/`create` before the daemon starts
workers and is read-only once applied) and **private** (a connection no other
connection can reach). Each row lists the opener as it now stands.

| Site | Function | Opener | Class |
| --- | --- | --- | --- |
| `ai/src/providers/mod.rs` | `atomically` | `BEGIN IMMEDIATE` (was `BEGIN`) | write |
| `imports/src/lib.rs` | `rollback` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `journals/src/prebuild.rs` | `atomic` | `begin_write_savepoint` (was `SAVEPOINT`; nested in the caller's writer) | write |
| `journals/src/persistence.rs` | `store_ledger_state_with_clock` | `begin_write_savepoint` (was `SAVEPOINT`; nested) | write |
| `journals/src/process.rs` | `build_in_transaction` | `BEGIN IMMEDIATE` + `SAVEPOINT journals_process` | write (already) |
| `journals/src/process/refresh.rs` | `refresh` | `BEGIN IMMEDIATE` | write (already) |
| `journals/src/review_apply.rs` | `begin` | `BEGIN IMMEDIATE` + `SAVEPOINT review_workflow_apply` | write (already) |
| `journals/src/review_tax.rs` | `tax_effects` | `begin_write_savepoint` (was `SAVEPOINT`; rolled back) | write |
| `journals/src/review_workflow.rs` | `plan` | `unchecked_transaction` on the in-memory clone | private |
| `journals/src/review_workflow.rs` | `inspect_effects`, `request_input` | `unchecked_transaction` | read |
| `kassiber/src/real_store.rs` | `custody_ai_write` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `kassiber/src/real_store/journals.rs` | `quarantine_resolve` | `BEGIN IMMEDIATE` | write (already) |
| `kassiber/src/real_store/rates_rebuild.rs` | `rates_rebuild` | `BEGIN IMMEDIATE` | write (already) |
| `rates/src/lib.rs` | `command` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `rates/src/lib.rs` | `begin_write` | `BEGIN IMMEDIATE` (was `BEGIN DEFERRED`) | write |
| `reports/src/export.rs` | `admitted` | `BEGIN IMMEDIATE`, or `SAVEPOINT` in a caller's transaction | write (already) |
| `reports/src/snapshot_write.rs` | `create_inner` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `reports/src/source_funds_batch.rs` | `execute_with_inputs` | `begin_write_savepoint` (was `SAVEPOINT`; dry run, rolled back) | write |
| `reports/src/source_funds_cases.rs` | `save_snapshot` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `reports/src/source_funds_suggest.rs` | `execute` | `begin_write_savepoint` (was `SAVEPOINT`; dry run) | write |
| `reports/src/privacy_mirror.rs` | `execute_with_index` | `begin_write_savepoint` (was `SAVEPOINT`; the durable index may publish) | write |
| `reports/src/lib.rs` | `execute_with_sources` | `SAVEPOINT report_read`, rolled back | read |
| `reports/src/artifact_xlsx.rs`, `artifact_csv.rs` | `prepare` | `SAVEPOINT`, rolled back | read |
| `reports/src/export_preview.rs` | `preview` | `SAVEPOINT`, rolled back | read |
| `reports/src/psbt_privacy.rs` | `report_with_baseline` | `SAVEPOINT` (in-memory index capture) | read |
| `reports/src/source_funds_review.rs` | `review_context` | `BEGIN`, rolled back | read |
| `secrets/src/credentials.rs` | `migrate_observed` | `write_transaction` (was `unchecked_transaction`) | write |
| `setup/src/lib.rs` | `execute` | `write_transaction`; `unchecked_transaction` for `reads_only` commands | write / read |
| `setup/src/backends.rs` | `Runtime::execute` | `write_transaction`; `unchecked_transaction` for `reads_only` operations | write / read |
| `setup/src/profile_reset.rs` | `execute` | `write_transaction` (was `unchecked_transaction`) | write |
| `setup/src/networks.rs` | `bind` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `setup/src/networks.rs` | `read_snapshot` | `unchecked_transaction`, rolled back (its contract discards an owned write, so it has no debug assertion) | read |
| `setup/src/network_partition.rs` | `export` | `BEGIN IMMEDIATE` (was `BEGIN`) on the staged new book | private |
| `setup/src/onboarding.rs` | `onboard` | `BEGIN IMMEDIATE` | write (already) |
| `setup/src/backend_runtime.rs` | `seed_inner` | `BEGIN IMMEDIATE` | write (already) |
| `store/src/lib.rs` | `Book::transaction` | `TransactionBehavior::Immediate` | write (already) |
| `store/src/lib.rs` | `with_savepoint` | `savepoint()` nested in `Book::transaction` | write (nested) |
| `store/src/lib.rs` | `open_with_policy` | `BEGIN DEFERRED` schema gate, rolled back | read |
| `store/src/observer.rs` | `snapshot_with` | `unchecked_transaction` | read |
| `store/src/creation.rs` | `install` | `TransactionBehavior::Immediate` | open (already) |
| `store/src/compat.rs` | `install_base` | `BEGIN IMMEDIATE` (was `BEGIN`) on an empty file | open |
| `store/src/compat.rs` | `install_graph` | `savepoint()` | open |
| `store/src/compat_ordered.rs` | `transaction_page_index` | `transaction()` | open |
| `store/src/compat_ordered.rs` | `import_batch_sequence` | `transaction()` (8f's import sequence) | open |
| `store/src/compat_schema.rs` | `migrate_recipient_unique`, `projection_shapes`, `economic_notes`, `immutability`, `commercial`, `step` | `savepoint()` | open |
| `store/src/compat_schema.rs` | `rebuild` | `TransactionBehavior::Immediate` | open (already) |
| `store/src/compat_backfill.rs` | `payment_hash`, `swap_fees`, `bip329`, `inbox`, `gap_inputs`, `swap_schema` | `savepoint()` | open |
| `store/src/compat_custody.rs` | `relation_schema`, `review_relations`, `evidence`, `durable_evidence` | `savepoint()` | open |
| `store/src/compat_liquid.rs` | `liquid` | `savepoint()` | open |
| `store/src/compat_ownership.rs` | `instance_id`, `inline_history` | `savepoint()` | open |
| `store/src/retired.rs` | `retire_ledger_with_observer` | `savepoint()` | open |
| `store/src/retired.rs` | `drop_device_sync_with_observer` | `TransactionBehavior::Immediate` | open (already) |
| `store/src/semantics.rs` | `migrate_selected`, `partial_payout` | `TransactionBehavior::Immediate` | open (already) |
| `store/src/stored_errors.rs` | `migrate` | `TransactionBehavior::Immediate` | open (already) |
| `transactions/src/sql.rs` | `atomic` | `begin_write_savepoint` (was `SAVEPOINT`; metadata, notes, tags, documents, loans) | write |
| `transfers/src/sql.rs` | `atomic` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `transfers/src/legacy_refresh.rs` | `atomic` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `transfers/src/pairing_commands.rs` | `execute` | `begin_write_savepoint` (was `SAVEPOINT`; dry run) | write |
| `transfers/src/pairing/service.rs` | `pending` | `BEGIN IMMEDIATE` (was `BEGIN`) | write |
| `transfers/src/review_cases.rs` | `inspect_with_error` | `unchecked_transaction` | read |
| `wallets/src/analysis_index/reads.rs` | `atomic` | `begin_write_savepoint` (was `SAVEPOINT`; the durable index may publish) | write |
| `wallets/src/analysis_index/views.rs` | `with_read` | `SAVEPOINT` | read |
| `wallets/src/analysis_index/sources.rs` | `build_index_at` | `SAVEPOINT` | read |
| `wallets/src/analysis_ops.rs` | `atomic` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `wallets/src/analysis_ops/sources.rs` | `run` | `BEGIN IMMEDIATE` (was `BEGIN`) | write |
| `wallets/src/btcpay/provenance.rs` | `atomic` | `begin_write_savepoint` (was `SAVEPOINT`; merged with connections) | write |
| `wallets/src/btcpay/setup.rs` | `apply_routes` | `begin_write_savepoint` (was `SAVEPOINT`; merged with connections) | write |
| `wallets/src/file_import/exchange_api.rs` | `execute` | `begin_write_savepoint` (was `SAVEPOINT`; merged with exchange sync) | write |
| `wallets/src/file_import/attachment.rs` | `plan_or_apply` | `begin_write_savepoint` (was `SAVEPOINT`; merged with the native AI gaps) | write |
| `wallets/src/file_import/document_chat.rs` | `plan_or_apply` | `begin_write_savepoint` for the plan and apply savepoints (was `SAVEPOINT`; merged with the native AI gaps) | write |
| `wallets/src/sync.rs` | `sync_local` | `write_savepoint` (was `savepoint()`; merged with connections) | write |
| `wallets/src/sync/btcpay.rs` | `nested` | `begin_write_savepoint` (was `SAVEPOINT`; merged with connections) | write |
| `wallets/src/analysis_ops/watches.rs` | `acknowledge` | `BEGIN IMMEDIATE` (was `BEGIN`) | write |
| `wallets/src/file_import.rs` | `run` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `wallets/src/file_import/samourai.rs` | `execute` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `wallets/src/file_import/document/coordinator.rs` | `import_draft` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `wallets/src/freshness/execution.rs` | `begin` | `BEGIN IMMEDIATE` (was `BEGIN DEFERRED`) | write |
| `wallets/src/freshness/settings.rs` | `configure` | `write_transaction` (was `unchecked_transaction`) | write |
| `wallets/src/lifecycle.rs` | `execute_with_creation_source` | `begin_write_savepoint` (was `SAVEPOINT`) | write |
| `wallets/src/publication.rs` | `with_wallet_savepoint` | `write_savepoint` (was `savepoint()`; wallet sync publication) | write |
| `wallets/src/publication/wallet.rs` | `apply_mode` | `write_savepoint` (was `savepoint()`) | write |

Paths are under `crates/`. The CLI, daemon, runtime and backup crates open no
transaction of their own. They reach these through the services above.
Openers in `#[cfg(test)]` code, examples and fixtures are out of scope.

`crates/custody/src/confirmation/liquid.rs::decodable` calls a Liquid
decoder's `transaction()`. It is not a database transaction, and the scanner
allowlist names it.
