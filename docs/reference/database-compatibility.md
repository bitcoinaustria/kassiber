# Database compatibility fixture matrix

This matrix defines the database-upgrade stop state for the custody-lineage
overhaul. “Tested” means an automated test opens the fixture with current code
and asserts the resulting behavior, not merely that schema creation succeeds.

| Compatibility concern | Before PR #432 (`5d232097`) | Before PR #435 (`16b7bdc1`) | Current schema | Deliberate compatibility rule |
| --- | --- | --- | --- | --- |
| Database integrity and current schema open | Tested: exact historical database, `PRAGMA integrity_check`; the upgrading connection reads `journal_custody_projection_relations` and a reopen changes no schema object | Tested: independent exact historical database, same single-open checks | Tested by fresh/current migration acceptance | Opening must be idempotent and must not reinterpret authored economics. The first open leaves a complete schema; it must not rely on a second open to repair it. |
| Manual transfer pairs | Tested: row, source, policy and amount survive | Tested: same | Tested end-to-end in `test_custody_migration_acceptance.py` | Explicit reviewed pairs survive. Graphless same-txid rows do not acquire equivalent authority automatically. |
| Conflicting custody components | Tested: both authored-active revisions survive and both remain ineffective | Tested: same | Tested with concurrent active revisions in `test_custody_migration_acceptance.py` | Conflicts fail closed; migration never picks a winner or silently supersedes a revision. |
| Samourai/Whirlpool missing-wallet gap | Tested: 10 BTC outbound and 9.9 BTC return remain a review candidate | Tested: same | Tested by the flagship full/missing Whirlpool variants | A likely bridge may block provisional tax output, but only explicit review carries basis. |
| Missing descriptors | Tested: Samourai metadata and transactions survive; no descriptor or policy epoch is invented | Tested: same | Tested by Samourai import and observer-boundary suites | Kassiber can identify incomplete observation and custody gaps. It cannot attest that every wallet was imported. Resync/re-import is required for authority. |
| Saved/filed reports | Tested: legacy journal rows survive and filed snapshots remain empty | Tested: same | Tested: saved/filed snapshots, impacts and immutable post-rebuild resolutions | Old journal/export activity is not retroactively called “filed.” Only an explicit filed assertion has that meaning. |
| Removed device-sync records | Tested: `sync_*` tables are dropped once with a `device-sync-removal-v1` audit of event and open-conflict counts; authored rows keep their values and uncommitted legacy active components remain ineffective | Tested: same | Tested: fresh books create no `sync_*` objects and the removal step only probes `sqlite_master` once applied | Device sync was removed (plan 19). Authored rows are never deleted; unused `transaction_edit_events.sync_*` columns stay in older books and are omitted from partitions. Missing modern evidence commitments are not reconstructed from mutable current rows. |
| Journals built before the pre-arbitration custody holds | Not applicable | Not applicable | Tested in `test_wages_semantic_migration.py` | `custody-fail-closed-holds-v1` marks every processed book stale once, so the next (automatic) rebuild applies the unscoped, fan-out, pending and replaced-spend holds. Retained journals and evidence are unchanged until then. |
| Journals built through RP2 | Not applicable | Not applicable | Tested in `test_wages_semantic_migration.py` | `kassiber-tax-engine-v1` marks every processed book stale once, so the next (automatic) rebuild uses Kassiber's own [tax engine](tax-engine.md) at one fixed decimal precision. Retained journals, evidence, and filed snapshots are unchanged until then. |
| Ancient REAL BTC to INTEGER msat rebuild | Tested with a focused legacy transaction/holdings fixture | Same migration path | Not applicable to a fresh current database | Amounts convert exactly and all later transaction metadata columns survive the rebuild. |
| Removed general-ledger storage (`gl_*`) | Not applicable | Not applicable | Tested in `test_legacy_ledger.py` | New databases get no `gl_*` objects. If every leftover table is empty, opening drops the ledger's `external_documents` triggers and then its tables; otherwise tables and triggers stay intact, the read-only probe writes nothing, and book deletion, reset, and partition fail with `legacy_ledger_present` until an explicit `kassiber maintenance purge-legacy-ledger --confirm`. |

Primary automated coverage:

- `tests/test_historical_custody_compatibility.py`
- `tests/test_msat_migration.py`
- `tests/test_custody_migration_acceptance.py`
- `tests/test_custody_lineage_flagship.py`
- `tests/test_device_sync_removal.py`
- `tests/test_custody_component_immutability.py`
- `tests/test_custody_filed_report_exports.py`
- `tests/test_observer_custody_boundaries.py`
- `tests/test_samourai_import.py`
- `tests/test_legacy_ledger.py`

## macOS full-sync durability

On macOS `fsync(2)` leaves data in the drive's cache. Both runtimes set
`PRAGMA fullfsync = ON` and `PRAGMA checkpoint_fullfsync = ON` there on every
book connection (plaintext and SQLCipher opens, every keyed SQLCipher open,
and the plaintext source of `secrets init`), so commits and WAL checkpoints
use `fcntl(F_FULLFSYNC)`. Other platforms keep SQLite's defaults. The pragmas
are per connection and change no stored byte, schema object or journal mode.
Python's file and directory syncs go through `kassiber/durable_sync.py`
(`F_FULLFSYNC` on macOS, falling back to `fsync` where a file system refuses
it, as SQLite does); Rust's `File::sync_all` already uses `F_FULLFSYNC`.
`tests/test_durable_sync.py` and the store's
`macos_connections_set_both_fullfsync_pragmas` check the macOS branch on any
host through a platform seam; `crates/store/scripts/durability_mutations.json`
lists the mutations they kill.

## Austrian consumed-regime journal migration

The `at-outflow-consumed-regime-v2` semantic migration supersedes the earlier
`at-fee-consumed-regime-v1` correction. It marks Austrian books with retained
disposal, fee or transfer-fee rows stale once and requires calculation semantics
version 2. A rebuild records the regime RP2 consumed before any taxability or
category overrides, splitting mixed regimes into separate rows. Books with
disposals but no fees are included. The migration retains observations, custody
evidence, journal rows and saved/filed artifacts; only processing replaces the
derived projection. Generic books receive no migration input-version bump.
Every processed book, including generic books, requires one rebuild after
upgrade because the global calculation semantics version increases to 2.
The targeted Austrian migration and the global version check are separate:
the latter also refuses older stamped builds after migration; an audit alone
cannot make journals rebuilt by an older writer fresh.

## Dependency-origin correction

The custody dependency-origin fixed-point correction changes only the
`blocked_by_transaction_ids` explanation stored in `journal_quarantines`:
previous builds could omit a connected explicit root depending on Python's
hash seed. Selected rows, blocked transaction membership, journal amounts and
report-readiness barriers are unchanged. No schema migration or automatic
journal invalidation is needed. Existing stored explanations remain until
`journals process` rebuilds them; review evidence and the displayed root may
then name a previously omitted blocker. Saved exports and filed snapshots
are not rewritten.

## Custody authoring transaction compatibility

The savepoint atomicity fix changes transaction ownership, not the schema or
existing authored revisions. Low-level component and legacy consolidation
helpers leave persistence to their caller; database initialization explicitly
commits consolidation changes, journal processing owns its writer transaction,
and review/component/gap authoring boundaries commit their completed operation.
No caller in these paths requires an outermost savepoint release to persist it.

`tests/test_custody_authoring_atomicity.py` injects component/gap failures after
the outermost authoring savepoint has actually been released, before the caller
commits. A failure inside that savepoint cannot expose an accidental release
commit. The suite compares
all persisted tables after rollback, including input versions, memberships and
audit records. It also pins rollback of successful `commit=False` applies.
There is no reliable historical failed-request marker from which to reconstruct
an owner's intended review. Opening an existing book does not silently repair
or replay authoring; see [recovery](custody-components.md#authoring-transactions-and-recovery).

## Compact retained calculations

Opening a book adds the nullable `journal_entries.calculation_compact` column
after the msat rebuild. Existing rows keep their `calculation_json` text and
are read as before. The next `journals process` stores each realized entry's
calculation in the compact column and leaves `calculation_json` NULL. A
calculation the compact form cannot restore exactly keeps the legacy column.
Explanations and `journals events get` return the same values and bytes from
either column. An older Python build ignores the column: it reports
compact rows' engine detail as unavailable until it rebuilds the journal
itself. An older Rust build refuses the newer schema. See
[storage-format speedups](r4-journal-pipeline.md#storage-format-speedups).

Journal entry identities now use the deterministic UUIDv8 construction in that
contract. Opening a book never rekeys it; its next rebuild replaces old uuid4
entries. Later rebuilds retain unchanged entries and quantity postings, including
their creation times, and rewrite a changed entry's whole timestamp group.
No new schema or migration audit is needed. The Rust store manifest and ordered
pass remain identical; both runtimes accept these opaque text ids. Older Python
can rebuild with uuid4 ids and legacy calculations, causing one full entry
rewrite the next time current Python rebuilds. Backups preserve both formats.

The scoped 128 MiB rebuild cache is a connection setting. It adds no schema,
migration or stored-format change, so the same Python/Rust schema manifest and
older-binary coexistence rules apply. SQLCipher retains DELETE/FULL durability.

Native journal persistence has a separate process-termination campaign in
`crates/journals/tests/crash.rs`: dirty-page DML boundaries and pre/post-COMMIT
on physical SQLite WAL/NORMAL and SQLCipher DELETE/FULL books. Reopen checks
complete typed dumps, integrity and foreign keys. This covers the unregistered
R4-12 writer, not migration termination or the R4-13 pipeline caller; see the
[persistence checkpoint](r4-journal-persistence-checkpoint.md#follow-up-from-b5eb38ea8).

## Intentionally not inferred during upgrade

### Journal calculation semantics and older writers

Python and native storage install the same semantics columns, adoption audit,
processing-marker trigger and version-2 outflow migration. Native retained-journal
readers apply the same calculation-stamp requirement as the Python core.

Journal builds now stamp `profiles.journal_semantics_version` separately after
publishing their input markers. A trigger clears that stamp whenever a writer
updates the processing markers, including an older binary that cannot stamp
its semantics. The update also requires a stamp for an older binary's first
build in a new book. Required semantics survive that update. Report builders
refuse unstamped or incompatible journals even when input versions and counts match.
Graph readers mark retained custody edges stale, and diagnostics count these
books as stale. Daemon report reads and maintenance rebuild locally through the
same canonical freshness check; they do not acquire network consent by doing so.
A reader missing either semantics column fails closed. Rebuilding with current
Kassiber restores freshness. First-upgrade adoption is keyed on the
`journal-calculation-semantics-adoption-v1` audit, independently of whether the
stamp columns already exist. It records the affected book count and required
version, and requires a stamped rebuild of previously processed books, because
the earlier custody-hold correction applied to every such book. Adoption leaves
unprocessed books alone and preserves input markers. Raw observations, authored
reviews, retained journals and filed snapshots are not rewritten by this guard.

This protects reopen after an older binary rebuilds; it cannot make an older
binary's own reports correct. Use the current binary for accounting. The exact
`ca9a672f` partial-payout fixture covers older publication both before and after
a current rebuild, including repeated publication at an identical timestamp.

The existing `wages-acquisition-semantics-v1`, `rp2-pool-basis-semantics-v1`,
`custody-fail-closed-holds-v1`, `at-acquisition-valuation-support-v1`, and
`custody-partial-payout-pricing-v1` migrations require stamped semantics
version 1; `at-outflow-consumed-regime-v2` requires version 2 for their affected books. Their
one-shot audits still record the upgrade, but are no longer the only protection
against accepting an older calculation. Future calculation changes must bump
the build semantics version and set the required version for affected books;
an audit sentinel alone cannot establish what generated retained journals.

### Retained authored facts

- Direct payout decimal precision lost through historical float conversion
  cannot be recovered. Existing authored terms are read as stored, including
  float-derived decimal tails; opening a book does not round or rewrite them.
  Use an explicit replacement review to correct the asserted economics.
- The partial direct payout projection correction changes rebuilt derived
  journals and live reports: residuals no longer inherit payout pricing or a
  carrying-value policy. The one-time `custody-partial-payout-pricing-v1`
  migration marks journals stale for exactly profiles with an active payout
  covering less than its whole source. Reports require `journals process`
  before using those journals again. This migration bumps input versions only
  for those profiles; initial semantics-stamp adoption above also requires
  previously processed books to rebuild. The
  `pre_partial_payout_ca9a672f` fixture was processed with pre-fix code and
  verifies refusal before rebuilding and corrected proceeds afterward.
  Missing residual prices can now block output rather than fabricate proceeds.
  Saved exports and filed snapshots remain unchanged; review any resulting
  report differences through the existing amendment workflow.

- A registered wallet or descriptor does not prove that the user imported the
  complete beneficial-ownership universe. Kassiber may clear technical
  quarantine for observed data; it cannot declare the book globally complete.
- A historical transaction is not upgraded to authoritative BDK/LWK evidence.
  Current observer provenance requires a current sync and matching committed
  graph/quantity hashes.
- A missing Whirlpool descriptor is not reconstructed from amounts, timing, or
  labels. Those signals can propose a gap; a reviewed bridge is the only action
  that carries basis across it.
- A legacy report-like row or exported file is not upgraded to a filed report.
  Filing is an explicit legal assertion, not a schema guess.
- An active component without its authored evidence commitments is not
  promoted. It stays visible and ineffective until valid evidence/review exists.

## Remaining fixture limits

- Rust's private ordered pass matches both original fixtures exactly up to
  the explicit R2 authored-consolidation refusal. Pair-free variants pass full
  first-pass parity and unchanged Python reopen after the custody view was
  moved after the leg-role rebuild. This does not establish full Rust migration
  of the original books; see the [R3 evidence](rust-storage-r3-status.md#ordered-pass-attempt).
- The checked-in fixtures are plaintext. R3 repeats those exact first-pass,
  prefix and reopen comparisons on disposable SQLCipher copies.
  These do not replace the separate SQLCipher page-format and passphrase tests.
- They do not simulate process termination at every migration statement;
  transaction/crash atomicity is covered by migration-specific tests rather
  than this scenario matrix.
- They do not prove that an unknown wallet was the user’s wallet. That fact is
  unknowable from a database alone and remains a user-reviewed boundary.


## Transaction page index

`idx_transactions_profile_page` is a nonunique covering index on
`transactions(profile_id, occurred_at, created_at, id, wallet_id)`. It changes
no row values, accounting authority, migration audit, or input version. Existing
indexes remain. Python fresh creation and the Rust creation transcript include
it; existing books defer creation from the base installer to the final ordered
compatibility step, after table rebuilds. The msat rebuild restores it too.
Python uses a savepoint and Rust a transaction. Both check the same stored
index definition and reject a conflicting definition without replacing it.
Unchanged creation is a no-op: no index rebuild, root-page change, or schema
cookie increment from this step. No `ANALYZE` or planner hints are used.

**Upgrade Rust before opening a book with the new Python build.** Rust at
`f261b9b2` rejects an additional index through its structural admission gate.
The gate remains strict. Older Python can ignore the additive index. Do not
remove the index to bypass admission; deploy the matching Rust build instead.

The page-index migration tests cover current books missing the index with
Python-first and Rust-first upgrades, plaintext and SQLCipher, complete typed
logical row dumps, subsequent opens, injected rollback and process termination.
The existing ordered matrix covers both historical fixtures and SQLCipher
copies: pair-free variants migrate in Rust; original authored-pair fixtures
retain their explicit R2 refusal and must first migrate in Python. This index
does not extend Rust's authored-consolidation support.

Python's existing compatibility pass recreates unrelated indexes, views and
triggers on every open (a fresh control book advances `schema_version` by 36).
This change does not address that existing behavior. Whole-open schema-object
and row equality and isolated page-index cookie/root-page stability are checked
separately; whole-open Python cookie stability is not claimed.

See the [page-index implementation evidence](transaction-page-index.md) for
measurements and verification results.

## Native journal pre-build legacy numeric inputs

R4-10 changes no schema or stored values on open. The unregistered pre-build
stage preserves historical REAL/text storage classes and the Python stage's
lazy quantity conversion. Malformed reviewed integer text propagates its exact
Python refusal and rolls back the stage; it does not silently become a skipped
migration. An oversized valid integer becomes the same exact blocking migration
issue as Python, without a native fixed-width panic. Existing fractional-msat
truncation is retained only for parity; see the [R4-10 edge evidence](r4-journal-pipeline.md#r4-10-legacy-and-refusal-expansion).

Binary numeric prices retain their stored BLOB class and Python's chained
decimal refusal during processing, including rollback of earlier pricing writes.
The private comparison dump preserves infinite REAL bits rather than substituting
null; this adds no database or product format. See the
[uncovered numeric paths](r4-journal-pipeline.md#r4-10-uncovered-numeric-paths).

## Storage added by main #675 to #708

Main after #654 added four storage changes. Python fresh creation includes
them; existing books receive them in `ensure_schema_compat`, and the Rust
store runs the same steps at the same positions of its ordered pass:

| Change | Existing books |
| --- | --- |
| `transactions.pricing_review_accepted_price` (#708) | Added after `pricing_quality`, and again after the msat rebuild, whose fixed column list drops it from a legacy REAL-typed table |
| `ai_providers.agent_access TEXT NOT NULL DEFAULT 'full'` | Added after `omit_store_field`; every existing provider stays `full` |
| `ai_provider_checks` | Created by the base schema, then by `_ensure_ai_provider_checks_schema` after the secret-reference table |
| Custody AI audit `consent_decision` CHECK widened to `mode_auto`, `mode_full_access`, `mcp_host` | Rebuilt once after the custody leg-role rebuild and before the projection view: foreign keys off, one immediate transaction, rows copied by Python's exact column list, row count checked (`custody_schema_migration_failed` if it changes), index and immutability trigger restored. A table that already admits all three values is left alone |

None of them writes a migration audit; they are schema steps, not semantic
migrations, so books keep the same ledger. A rebuilt or altered table keeps
the migration's own SQL spelling, as in Python, and the structural admission
gate accepts it.

Coverage: `pre_sync_bae01537b.sqlite3.gz.b64` (a processed Austrian book
with an AI provider and narrow-CHECK audit rows, written by main #654) joins
the historical fixtures, so the exact first-pass dump, Python reopen and Rust
re-migration comparisons run on it in plaintext and SQLCipher.
`tests/test_rust_store_migrations.py` also compares the audit rebuild with
Python's step, kills it at each checkpoint and resumes in either runtime,
opens a current Python book holding rows in every new object with Rust (both
open modes) without any change, and alternates Rust and Python opens of a
Rust-created book with no migration loop.
`crates/store/scripts/mutations.json` breaks each step and the refusal rules
one at a time (`crates/store/scripts/mutation_check.py`); every entry fails
a Rust store test.

Rust tests that rebuild their books from the manifest carry frozen Python
dumps cut from the older schema. `tests/rust_port/upgrade_schema_dumps.py`
upgraded them for these steps, filling in each added column's default and
the empty table, as this compatibility pass does. A later additive column
can extend its `ADDED` list. A step that backfills other values, drops a
column or reorders one needs its vectors regenerated instead. Whole-book
checks compare their table names with `kassiber_store::table_names()` or
`tests/rust_port/book_schema.py`, not with a fixed count.

**Newer layouts.** The Rust store refuses a book carrying any object, column
or migration audit it does not know (`requires_python_migration`), in both its
admission-only and migrating opens, before writing anything. Python does not:
its open is additive and leaves unknown tables, columns, indexes, triggers
and audits in place. The asymmetry is deliberate. A Rust binary must not run
accounting on a schema written by a newer build, and an older Python build
keeps its existing behaviour. Upgrade the Rust binary together with Python.
