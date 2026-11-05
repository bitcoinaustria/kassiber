# Rust port verification inventory

R0 inventories Python tests before moving each [plan 21](../plan/21-rust-backend.md)
service group. The [contract inventory](rust-port-contracts.md) records startup,
storage, environment and wire details that a replacement can easily miss.
Neither inventory is a claim that a scenario already runs on Rust.

`tests/rust_port/dispositions.json` assigns every discovered `test_*.py` module,
test class and module-level test function. Mixed classes retain a default plus
`Class::test_method` exceptions; `tests` counts only methods covered by that
entry, so exceptions are never counted twice. Empty class defaults can have
zero tests. Parametrized functions count once, not once per parameter set.
Pytest collection was cross-checked against the AST census: all collected
definition IDs and module totals matched, including inherited methods.
Existing Rust and Vitest suites are outside this Python census.

| Disposition | Migration action |
| --- | --- |
| `boundary-fixture` | Record a real CLI/daemon/MCP/broker child session with external inputs; eligible for the planned KASSIBER_TEST_CLI path. No mocked process/product dependencies or raw SQL mutation in the reachable setup/helpers. |
| `subprocess-scenario` | Rewrite a boundary-observable test currently using in-process entry/dispatch or Python/SQL setup into a binary scenario. |
| `rust-test` | Port service/unit/property intent to the owning Rust crate. Repository/packaging assertions may remain in repository tooling during migration and eventually move to a native check. |
| `oracle` | Retain independent implementation comparisons, reference goldens/vectors and migration contract checks. |
| `python-only` | Retire interpreter-specific mechanics; each entry names the replacement for its intent, including native guard/dependency/packaging checks. |

Flags are conservative static evidence over reachable test helpers and setup:
`mock` covers patching other than environment/test-helper replacement,
`mock-internal` specifically identifies product patches, `sql` covers raw SQL,
sqlite3 or open_db, `sql-seed` identifies mutating SQL, `tapes` identifies tape
use, and `subprocess` identifies child processes (not necessarily the product).
A SQL read assertion does not itself seed a book. Static inspection cannot
prove every dynamic alias/fixture path, and eligibility still needs clean
Python-to-Python replay before becoming an accepted recording.

The AST classifier follows local/imported test helpers, unittest setup and
inheritance and module fixtures; module/group naming is a heuristic.
`tests/rust_port/reviews.py` contains explicit corrections and concrete sample
notes for the 30 largest modules by line count, including setup and helper
paths and first/middle/last tests. Mixed CLI/daemon classes retain method-specific
entries. A sampled module is not marked wholly reviewed: `reviewed:true`
means that entry has an explicit override. Examples of corrected mistakes are
SQL-seeded subprocesses mislabeled portable, tax preparation comparisons
mislabeled units, and schema tests assigned to custody instead of store.

Regenerate deterministically from the repository root:

```sh
uv run --locked python -m tests.rust_port.classify_tests update
uv run --locked python -m tests.rust_port.classify_tests check
uv run --locked python -m tests.rust_port.classify_tests summary
uv run --locked python -m tests.rust_port.contracts update
uv run --locked python scripts/python_test_shards.py run preflight -- -q
uv run --locked python scripts/python_test_shards.py validate
```

Use `classify_tests explain tests/test_example.py` for pre-review evidence.
Coverage validation fails for absent/stale modules/classes/functions/method
exceptions, invalid dispositions, missing reasons/replacements and changed
class method counts. Regenerate after extending a class; review new heuristic
decisions. It does not silently exempt new modules. The count table below is
the classifier's `summary` output and is checked by preflight.

<!-- disposition-counts:start -->
| Group | boundary-fixture | subprocess-scenario | rust-test | oracle | python-only | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| store | 2 | 13 | 174 | 62 | 0 | 251 |
| custody | 0 | 38 | 389 | 138 | 1 | 566 |
| journals | 4 | 14 | 219 | 230 | 8 | 475 |
| imports | 67 | 33 | 171 | 48 | 0 | 319 |
| reports | 56 | 36 | 226 | 158 | 0 | 476 |
| chain/sync | 38 | 76 | 1280 | 153 | 0 | 1547 |
| network/egress | 7 | 15 | 81 | 35 | 10 | 148 |
| transactions/metadata/transfers/reviews | 46 | 103 | 541 | 54 | 0 | 744 |
| source-funds | 51 | 50 | 139 | 0 | 0 | 240 |
| secrets/backup | 7 | 18 | 81 | 56 | 0 | 162 |
| daemon runtime | 8 | 25 | 222 | 110 | 1 | 366 |
| AI/MCP/operator | 64 | 119 | 628 | 47 | 0 | 858 |
| CLI shell | 20 | 0 | 30 | 166 | 3 | 219 |
| packaging/CI | 0 | 2 | 166 | 10 | 29 | 207 |
| **Total** | **370** | **542** | **4347** | **1267** | **52** | **6578** |
<!-- disposition-counts:end -->

Plan 21's five migration choices include Python-only retirement explicitly;
its previous four-choice prose omitted this category. The command/kind scale
counts remain accurate at the base, while desktop streaming declarations have
one existing difference detailed in the contract inventory. None of these
findings changes the requirement to prove native accounting/security parity.
