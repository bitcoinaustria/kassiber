# Database compatibility fixture matrix

This matrix defines the database-upgrade stop state for the custody-lineage
overhaul. “Tested” means an automated test opens the fixture with current code
and asserts the resulting behavior, not merely that schema creation succeeds.

| Compatibility concern | Before PR #432 (`5d232097`) | Before PR #435 (`16b7bdc1`) | Current schema | Deliberate compatibility rule |
| --- | --- | --- | --- | --- |
| Database integrity and current schema open | Tested: exact historical database, `PRAGMA integrity_check` | Tested: independent exact historical database | Tested by fresh/current migration acceptance | Opening must be idempotent and must not reinterpret authored economics. |
| Manual transfer pairs | Tested: row, source, policy and amount survive | Tested: same | Tested end-to-end in `test_custody_migration_acceptance.py` | Explicit reviewed pairs survive. Graphless same-txid rows do not acquire equivalent authority automatically. |
| Conflicting custody components | Tested: both authored-active revisions survive and both remain ineffective | Tested: same | Tested with current replicated revisions and lifecycle conflicts | Conflicts fail closed; migration never picks a winner or silently supersedes a revision. |
| Samourai/Whirlpool missing-wallet gap | Tested: 10 BTC outbound and 9.9 BTC return remain a review candidate | Tested: same | Tested by the flagship full/missing Whirlpool variants | A likely bridge may block provisional tax output, but only explicit review carries basis. |
| Missing descriptors | Tested: Samourai metadata and transactions survive; no descriptor or policy epoch is invented | Tested: same | Tested by Samourai import and observer-boundary suites | Kassiber can identify incomplete observation and custody gaps. It cannot attest that every wallet was imported. Resync/re-import is required for authority. |
| Saved/filed reports | Tested: legacy journal rows survive and filed snapshots remain empty | Tested: same | Tested: saved/filed snapshots, impacts and immutable post-rebuild resolutions | Old journal/export activity is not retroactively called “filed.” Only an explicit filed assertion has that meaning. |
| Replicated records | Tested: events and open conflict survive; uncommitted legacy active components remain ineffective | Tested: same | Tested across v1 relation replay, missing commitments, concurrent revisions and filed-impact convergence | Signed history is preserved. Missing modern evidence commitments are not reconstructed from mutable current rows. |
| Ancient REAL BTC to INTEGER msat rebuild | Tested with a focused legacy transaction/holdings fixture | Same migration path | Not applicable to a fresh current database | Amounts convert exactly and all later transaction metadata columns survive the rebuild. |

Primary automated coverage:

- `tests/test_historical_custody_compatibility.py`
- `tests/test_msat_migration.py`
- `tests/test_custody_migration_acceptance.py`
- `tests/test_custody_lineage_flagship.py`
- `tests/test_custody_component_replication.py`
- `tests/test_custody_filed_report_exports.py`
- `tests/test_observer_custody_boundaries.py`
- `tests/test_samourai_import.py`

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
- A received active component without its authored evidence commitments is not
  promoted. It stays visible and ineffective until valid evidence/review exists.

## Remaining fixture limits

- The historical fixtures cover plaintext schema/data migration. They do not
  duplicate the separate SQLCipher page-format and passphrase tests.
- They do not simulate process termination at every migration statement;
  transaction/crash atomicity is covered by migration-specific tests rather
  than this scenario matrix.
- They do not prove that an unknown wallet was the user’s wallet. That fact is
  unknowable from a database alone and remains a user-reviewed boundary.

