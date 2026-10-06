# Frozen tax oracle

Phase 4 of [plan 20](../../docs/plan/20-kassiber-tax-engine.md) moves the
tax-ledger fold into the Rust engine. Production must give exactly what this
oracle gives: the phase-3 adapter, frozen together with the code it folds
with, running on RP2. `tests/tax_engine_compare.py` compares the two, and
`KASSIBER_TEST_TAX_ENGINE` selects how a test session uses them
([testing](../../docs/reference/testing.md#tax-engine-backends)).

| File | Content |
| --- | --- |
| `adapter.py` | `kassiber/core/engines/rp2.py`, exposing only `OracleTaxEngine` |
| `normalize.py` | `kassiber/core/tax_events.py`, including its `build_tax_quarantine` |
| `austrian.py` | `kassiber/core/austrian.py`: regime tracker, constants, Kennzahl |
| `pair_allocation.py` | `kassiber/core/pair_allocation.py` |
| `policy.py` | the oracle's `build_tax_policy` |
| `projection_range.py` | the shapes a finalized projection cannot have (LD-5) |
| `FROZEN.json` | SHA-256 of each copied file after the edits below |
| `test_oracle_contract.py` | the checks that keep the oracle what it claims to be |

The four copies were taken at commit `540c21b8`.

## Allowed edits

These are the only differences between a copy and its source at the freeze
commit:

- Relative imports become absolute: `kassiber.*` for the product modules
  that define the oracle's input or helpers (`errors`, `msat`, `tax_policy`,
  `transfers`, `pricing`, `quarantine_catalog`, `transaction_kinds`,
  `transfer_chronology`, `ownership_transfers`, `journal_markers`, `loans`,
  `onchain`, `privacy_hops`, `transfer_matching`, `custody_tax_projection`,
  `engines.base`, `engines.explanation`, and `engines.native` inside
  `_native_backend`). `tax_events`, `austrian`, and `pair_allocation` become
  the copies here.
- `build_tax_policy` comes from `tests.tax_oracle.policy`;
  `GLOBAL_COST_BASIS_POOL_ID` and `require_tax_processing_supported` still
  come from `kassiber.tax_policy`.
- `GenericRP2TaxEngine` is renamed `OracleTaxEngine`, without an alias. Its
  error messages keep the old name, because they are output.
- The adapter's default backend is `rp2` instead of `native`, and the comment
  above it says so. `_use_tax_engine_backend("native")` selects the
  legacy-native leg.

`policy.py` is not a copy: its generic half repeats the product's
`build_generic_policy` with its constants, and Austria comes from RP2's AT
plugin on `rp2` and from `native.AT()` on legacy-native, following this
adapter's backend switch, never the product's.

## Freeze rule

Any other edit is a bug fix. It lands in production and in the oracle in the
same commit, with a fixture that shows it, and updates `FROZEN.json`.

The product modules imported above are shared. A change in one of them moves
the oracle and production together, so a shadow run cannot see it; the
stored oracle outcomes of the projection-level goldens can.

Only the comparison harness, this package, the ledger goldens and prepare
tests, `tests/conftest.py`, the child-process hook and its harness test, the
interface-mirror tests, and the fixture generator may import
`tests.tax_oracle` or `tests.tax_engine_compare`, which runs it;
`tests/test_tax_oracle_imports.py` holds the list.
