# Database compatibility fixture matrix

This matrix defines the database-upgrade stop state for the custody-lineage
overhaul. “Tested” means an automated test opens the fixture with current code
and asserts the resulting behavior, not merely that schema creation succeeds.

| Compatibility concern | Before PR #432 (`5d232097`) | Before PR #435 (`16b7bdc1`) | Current schema | Deliberate compatibility rule |
| --- | --- | --- | --- | --- |
| Database integrity and current schema open | Tested: exact historical database, `PRAGMA integrity_check` | Tested: independent exact historical database | Tested by fresh/current migration acceptance | Opening must be idempotent and must not reinterpret authored economics. |
| Manual transfer pairs | Tested: row, source, policy and amount survive | Tested: same | Tested end-to-end in `test_custody_migration_acceptance.py` | Explicit reviewed pairs survive. Graphless same-txid rows do not acquire equivalent authority automatically. |
| Conflicting custody components | Tested: both authored-active revisions survive and both remain ineffective | Tested: same | Tested with concurrent active revisions in `test_custody_migration_acceptance.py` | Conflicts fail closed; migration never picks a winner or silently supersedes a revision. |
| Samourai/Whirlpool missing-wallet gap | Tested: 10 BTC outbound and 9.9 BTC return remain a review candidate | Tested: same | Tested by the flagship full/missing Whirlpool variants | A likely bridge may block provisional tax output, but only explicit review carries basis. |
| Missing descriptors | Tested: Samourai metadata and transactions survive; no descriptor or policy epoch is invented | Tested: same | Tested by Samourai import and observer-boundary suites | Kassiber can identify incomplete observation and custody gaps. It cannot attest that every wallet was imported. Resync/re-import is required for authority. |
| Saved/filed reports | Tested: legacy journal rows survive and filed snapshots remain empty | Tested: same | Tested: saved/filed snapshots, impacts and immutable post-rebuild resolutions | Old journal/export activity is not retroactively called “filed.” Only an explicit filed assertion has that meaning. |
| Removed device-sync records | Tested: `sync_*` tables are dropped once with a `device-sync-removal-v1` audit of event and open-conflict counts; authored rows keep their values and uncommitted legacy active components remain ineffective | Tested: same | Tested: fresh books create no `sync_*` objects and the removal step only probes `sqlite_master` once applied | Device sync was removed (plan 19). Authored rows are never deleted; unused `transaction_edit_events.sync_*` columns stay in older books and are omitted from partitions. Missing modern evidence commitments are not reconstructed from mutable current rows. |
| Journals built before the pre-arbitration custody holds | Not applicable | Not applicable | Tested in `test_wages_semantic_migration.py` | `custody-fail-closed-holds-v1` marks every processed book stale once, so the next (automatic) rebuild applies the unscoped, fan-out, pending and replaced-spend holds. Retained journals and evidence are unchanged until then. |
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

## Intentionally not inferred during upgrade

### Journal calculation semantics and older writers

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
Kassiber restores freshness. The first upgrade also
requires a stamped rebuild of previously processed books, because the earlier
custody-hold correction applied to every such book. Raw observations, authored
reviews, retained journals and filed snapshots are not rewritten by this guard.

This protects reopen after an older binary rebuilds; it cannot make an older
binary's own reports correct. Use the current binary for accounting. The exact
`ca9a672f` partial-payout fixture covers older publication both before and after
a current rebuild, including repeated publication at an identical timestamp.

The existing `wages-acquisition-semantics-v1`, `rp2-pool-basis-semantics-v1`,
`custody-fail-closed-holds-v1`, `at-acquisition-valuation-support-v1`, and
`custody-partial-payout-pricing-v1` migrations all require this stamped semantics
version for their affected books. Their
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

