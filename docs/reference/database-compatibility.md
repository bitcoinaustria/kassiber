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
| Pair components authored before observed-fee separation | Not applicable | Not applicable | Tested against an exact `c0a015de` book in `test_custody_pair_fee_compatibility.py`, including interrupted invalidation and repeat opens | `custody-legacy-pair-fee-review-v1` audits counts and marks affected journals stale once. A source below the complete observed debit with a `reviewed_residual:network_fee` allocation fails validation with `legacy_pair_fee_allocation_review_required`. The immutable component stays visible for revision; migration does not reinterpret its fee or suspense. Audit and invalidation roll back together. |
| Journals built with a random same-time lot order | Not applicable | Not applicable | Tested in `test_lot_order_determinism.py` | `same-time-source-order-v1` requires calculation semantics version 3 of every processed book, once, so the next (automatic) rebuild orders same-time rows by the [source order](tax.md#same-time-order). Input markers, retained journals and filed snapshots are unchanged until then. |
| Ancient REAL BTC to INTEGER msat rebuild | Tested with a focused legacy transaction/holdings fixture | Same migration path | Not applicable to a fresh current database | Amounts convert exactly and all later transaction metadata columns survive the rebuild. |
| Credentials repeated in stored error text | Not applicable | Not applicable | Tested in `test_credential_echo.py` and `test_rust_store_migrations.py` | `stored-error-credentials-v1` withholds, once, every credential the book holds from the freshness, BTCPay store-sync, Lightning-sync and AI provider-check error columns; JSON columns are rewritten compactly. Python and the Rust store's migrating open produce the same bytes and audit. A credential no longer in the book cannot be recognized ([credential echoes](credential-echo.md)). |
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
## ID-less CSV identity compatibility

Binance supplemental autoinvest/dividend, Coinfinity, Pocket Bitcoin and Ledger
Live previously included the export's row position in fallback transaction IDs.
They now use an economic hash plus the occurrence count of identical rows in
that export. Explicit provider and chain IDs keep their original identity.

Import recomputes the economic hash of recognizable stored synthetic rows from
their `raw_json`, separately per wallet. A stored current ID holds exactly the
occurrence number in its suffix. Old positional rows, ordered unbatched first,
then by import order and old numeric row index, fill only the lowest
occurrence numbers no current row holds. Incoming occurrence *n* is already
present when a stored row, including an excluded one, holds *n*. A rolled-back
first import therefore leaves its occurrence free for the next re-import
instead of letting a later occurrence stand in for it. The exception is an
excluded cross-batch copy, old or current: when a group holds more rows than its
largest single-import count, that many excluded rows (latest import first) hold
no number, so a reviewed copy cannot stand in for a later genuine repeat. The
compatibility match changes no stored IDs, fingerprints, raw observations, batch
memberships, or reviews. Overlapping exports can add new rows and genuinely
repeated occurrences without reopening reviewed gaps.
Missing or malformed source payloads and explicit provider IDs are not guessed
into this compatibility mapping. An old ID qualifies when its stored raw row has
no provider ID, it starts with the old prefix and raw date, and it ends in a
positive row index; earlier amount spellings do not matter. Old Binance
dividend IDs (`binance:<index>`) also need text-only CSV cells, because the API
importer writes `binance:<epoch>` for an ID-less dividend with numeric cells.

Import order is `import_batches.import_sequence`, not `imported_at`, which has
one-second resolution, nor the random batch ID. An insert trigger numbers every
new batch, including one an older binary writes. Opening an existing book
numbers its batches once by `(imported_at, rowid)`, the best insertion evidence
it holds; batches already in the same second keep their rowid order. Partitions
and SQLCipher exports copy the stored number, so later rowid changes do not
reorder imports. The msat rebuild does not touch import batches.

Journal calculation semantics version 4 detects existing excess copies by
wallet and economic hash. Rows without an import batch predate batch recording
(batches landed after these importers shipped), so all unbatched rows count as
one more import beside the recorded batches. If active rows span at least two
such imports and exceed the largest single-import count, every active member
gets the report-blocking `synthetic_import_duplicate_review_required` review
item. Excluded members still establish the original single-export count. A
current occurrence suffix also establishes that count when its batch inserted
only the new remainder of an overlapping export. One export's genuine identical
repeats remain valid. Version 3 ignored unbatched copies, so a duplicate whose
first copy predated batches escaped the hold.

Version 5 also holds a group when an active current row took an occurrence
number that, among earlier imports, only an excluded copy had held: the
exclusion does not say whether that copy was the same trade, so the owner
excludes the new row or confirms distinct trades, which counts the excluded
trade again through the new row. Version 6 checks this even when every earlier
copy is excluded, orders imports by sequence, and compares each earlier copy's
number at its own import, so an old row renumbered around a later current ID
does not cause a hold. Version 6 also fails closed when every active row
predates batches: such rows count as one export only when stored in the same
`created_at` second, and the group is held when its active rows exceed the
largest such count. One old import that crossed a second boundary is the known
false hold; confirming distinct trades clears it and nothing is merged. The
quarantine detail and review case list `hold_reasons` (`cross_import_excess`,
`refilled_excluded_copy`, `pre_batch_excess`) and
`refilled_excluded_transaction_ids`.

The detector excludes and deletes nothing and chooses no survivor. Review the
exports, use `metadata exclude --transaction <id>` for the copies the user
identifies, and rebuild journals. When every held row is a genuine separate
trade (for example, an older build recorded only part of a batch that held real
identical repeats), confirm that instead through `review plan`/`review apply`
with `{"type": "confirm_distinct_import_rows", "transaction_id": "<held id>",
"expected_fingerprint": "<group_fingerprint>", "reason": "..."}`. The case and
quarantine detail carry the `group_fingerprint`. Apply appends a
`synthetic_import_duplicate_reviews` row bound to the group's exact members and
their exclusion state, then invalidates journals; it changes no transaction.
A stale fingerprint returns `review_case_changed`, a replayed plan fails its
input-version check, and any later change to the group (another copy, a removed
or re-included row) re-opens the hold. The AI boundary refuses the operation
with `interaction_required`. Earlier version-2 through version-5 journals
require a current rebuild before report use; retained journals, authored reviews and saved/filed
snapshots are unchanged until their normal explicit processing/review steps.

Coverage: `tests/test_import_csv_upgrade_compatibility.py` restores exact rows
written by the pre-change parsers/storage writer and checks all five families,
overlapping exports, genuine repeats, reviewed gap evidence, cross-batch review,
copies that predate batches (including the per-second fail-closed rule),
rolled-back imports, either choice of excluded copy, a genuine repeat after an
excluded copy, refills when every earlier copy is excluded, imports in the same
second, the batch sequence migration and encryption, the confirmed-distinct review
and its binding to exclusion state, and a network partition that must carry
import batches with their rows so a held group stays held.

## Same-time source order

Opening a book adds the nullable `import_batch_transactions.source_position`
column (after `journal_entries.calculation_compact`) and the base schema's
`idx_import_batch_transactions_transaction` index on its `transaction_id`.
Existing links keep a NULL position, so their rows sort before positioned
rows of the same time and among themselves by content; rows created by later
file imports record their place in the run. No transaction row changes.

The global calculation semantics version becomes 3 and the
`same-time-source-order-v1` audit raises every processed book's required
version to 3 without touching its input markers. Freshness then reports
"journal calculation semantics changed; rebuild with current Kassiber" until a
version-3 rebuild, which may book different gains, cost basis or fallback
rates where same-time rows were ordered by row id before. An older build that
rebuilds stamps an older version, which stays stale for current readers.

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

`tests/test_custody_authoring_atomicity.py` injects late failures and compares
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
Kassiber restores freshness. The first upgrade also
requires a stamped rebuild of previously processed books, because the earlier
custody-hold correction applied to every such book. Raw observations, authored
reviews, retained journals and filed snapshots are not rewritten by this guard.

This protects reopen after an older binary rebuilds; it cannot make an older
binary's own reports correct. Use the current binary for accounting. The exact
`ca9a672f` partial-payout fixture covers older publication both before and after
a current rebuild, including repeated publication at an identical timestamp.

Calculation semantics version 2 includes the legacy pair fee validation blocker.
The pair migration requires version 2 for affected books, so a version-one
writer cannot clear the review barrier by publishing matching input markers.
Previously stamped version-one journals also require a current rebuild under
the shared freshness check. Retained journals and filed snapshots stay intact.

The existing `wages-acquisition-semantics-v1`, `rp2-pool-basis-semantics-v1`,
`custody-fail-closed-holds-v1`, `at-acquisition-valuation-support-v1`, and
`custody-partial-payout-pricing-v1` migrations require stamped semantics
version 1; `at-outflow-consumed-regime-v2` requires version 2 for their affected books,
and `same-time-source-order-v1` version 3 for every processed book. Their
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

- The historical fixtures cover plaintext schema/data migration. They do not
  duplicate the separate SQLCipher page-format and passphrase tests.
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
