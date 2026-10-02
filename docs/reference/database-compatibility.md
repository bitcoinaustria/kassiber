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

## Intentionally not inferred during upgrade

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

