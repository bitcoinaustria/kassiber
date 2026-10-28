# R2c producer composition checkpoint

Branch `t3code/r2c-interpretation-8` starts at `98cb0fb2`. Consolidation production
was verified and committed separately as `1ece1b1b` before this composition slice.
Python remains authoritative, snapshot v1 contains raw facts only, and the
product still includes `interpretation` in `skipped_stages`.

## Composition boundary

`interpretation/pipeline.rs` ports `derive_profile_transfers` in Python's exact
consolidation -> recorded fan-out -> ownership order. Each downstream producer
sees surviving rows in their original order, replacements at the same positions,
and synthetic rows appended in producer order. Drops precede overrides. Handled
ids include caller reservations, dropped consolidation/fan-out rows, fan-out
overrides and real fan-out receipt ids.

The new `partial.interpretation.pipeline` comparison returns all three complete
derivations, intermediate row populations, initial reservations, annotated pairs,
generic claims, rowless native audits and producer quarantines. Its compiler
reserves complete recorded pairs and Samourai rows, and consumes canonical exact
principal/fee facts for raw implicit wallet deltas. That normalization is internal
output, never a supplied snapshot verdict. Ownership residual annotations use
the journal anchor and preserve existing values with Python `setdefault`
semantics. Compiler candidates follow consolidation -> ownership -> fan-out,
matching the interpreter rather than producer execution order.

Producer blocks expand across sorted canonical event aliases, deduplicate by
transaction/reason, preserve existing atomic detail and use each member's scope
facts. Authored excluded members remain exempt individually; their unreviewed
siblings retain quarantine. Detail strings use sorted CPython JSON. The shared
CPython casing helpers and corrected timestamp parser remain in place. There
are no new dependencies or raw `codec::wide()` calls.

This remains a partial compiler. Native transition reservations, channel pairs,
holds/privacy, loans, reconciliation and the complete interpreter output are
outside it. A partial claim does not authorize product accounting.

## Witnesses and successful controls

There are **319 direct composition witnesses**, **14 helper witnesses** (five
row populations, three residual annotations and six quarantine cases), and
**215 raw snapshot projections** in the complete partial fixture. Direct vectors
keep normalized producer rows outside snapshot `input`; frozen expectations are
regenerated independently through Python. A graphless fan-out source with real
receipt funding evidence makes receipt reservations observable to ownership.

The composed generator compares **2,000 raw books and 24 permutations**. Every
success count below is asserted separately for **Python and Rust**; each cell
is `Python / Rust`. Consolidation and fan-out success each require two pairs
and two compiler claims. Ownership success requires one pair and one native
audit. No successful book has a producer or compiler blocker/quarantine.

| Scenario | Consolidation | Fan-out | Ownership | Blocked books |
| --- | ---: | ---: | ---: | ---: |
| `bitcoin_consolidation` | 167 / 167 | 0 / 0 | 0 / 0 | 0 / 0 |
| `liquid_consolidation` | 167 / 167 | 0 / 0 | 0 / 0 | 0 / 0 |
| `bitcoin_fanout` | 0 / 0 | 167 / 167 | 0 / 0 | 0 / 0 |
| `liquid_fanout` | 0 / 0 | 167 / 167 | 0 / 0 | 0 / 0 |
| `bitcoin_ownership` | 0 / 0 | 0 / 0 | 167 / 167 | 0 / 0 |
| `liquid_ownership` | 0 / 0 | 0 / 0 | 167 / 167 | 0 / 0 |
| `bitcoin_mixed` | 167 / 167 | 167 / 167 | 167 / 167 | 0 / 0 |
| `liquid_mixed` | 167 / 167 | 167 / 167 | 167 / 167 | 0 / 0 |
| `bitcoin_normalized` | 166 / 166 | 0 / 0 | 0 / 0 | 0 / 0 |
| `bitcoin_collaborative` | 166 / 166 | 0 / 0 | 0 / 0 | 0 / 0 |
| `bitcoin_residual` | 0 / 0 | 0 / 0 | 166 / 166 | 0 / 0 |
| `bitcoin_blocked` | 0 / 0 | 0 / 0 | 0 / 0 | 166 / 166 |
| Total | **1,000 / 1,000** | **668 / 668** | **834 / 834** | **166 / 166** |

Each blocked book has two atomic producer quarantines and no compiler claim or
audit. The first 24 controls also compare claims and audits with the full Python
interpreter, independently of the partial wrapper. The earlier graph controls
still assert **250 successes per engine per scenario** for Bitcoin/Liquid
consolidation, Bitcoin/Liquid ownership and all four Liquid raw-hex variants;
see the [consolidation checkpoint](custody-r2c-consolidation.md).

## Branch and mutation accounting

| Mutation group | Entries | Named-test result |
| --- | ---: | --- |
| Retained decoder/graph/interpreter/component/evidence rules | 389 | All killed |
| Retained stored-mapping admission rules | 12 | All killed |
| Retained graph-arithmetic rules | 37 | All killed |
| Retained ownership production/helpers | 120 | All killed |
| Consolidation production/helpers | 84 | All killed |
| Producer composition/compiler | 39 | All killed |
| Total | **681** | **All killed** |

The retained 642 kills were established before the composition commit. The 39
new entries initially ran in disjoint groups of 20 and 19, each passing its
named tests unmutated, and a final complete 39-entry rerun kills them all again.
Final applicability validation covers the whole catalog.
No survivor or nonbuilding mutation is counted as a kill. Exact ids, patches
and named tests are in `crates/custody/scripts/mutations.json`.

| New branch family | Entries | Named Rust test |
| --- | ---: | --- |
| Row dropping/replacement/appending, residual annotations, alias expansion and quarantine facts | 23 | `pipeline_helper_vectors` |
| Initial reservations, producer invocation/order, intermediate population and fan-out receipt reservations | 6 | `pipeline_production_vectors` |
| Complete recorded/Samourai reservations, chain eligibility, exact fee normalization and exception family | 10 | `interpretation_pipeline_projection` |

The following equivalences and unreachable branches do not introduce additional
product decisions:

- The journal calls `derive_profile_transfers` with `sort_key=None`. Arbitrary
  Python callback sorting belongs to other callers and is outside this seam.
- Consolidation and fan-out currently produce no `out_row_overrides`. The shared
  row helper nevertheless preserves replacement and drop-before-override behavior
  with direct witnesses and killed mutations.
- Handled additions for already dropped consolidation/fan-out rows are redundant
  in current producers: those rows leave the population and synthetic ids are
  distinct. The exact Python updates remain. Fan-out's real receipt reservation
  is independently observable and mutation-killed by the graphless-source case.
- Producer pairs and blocks have internally constructed object shapes; converting
  fan-out's fixed derivation into the shared typed result cannot fail on admitted
  raw facts. Producer detail is always an object. Missing reason, missing event,
  missing alias-row scope and preexisting atomic/residual defaults are pinned by
  helper witnesses; these helpers never add fields to snapshot v1.
- Raw snapshot ids are strings. Empty ids and absent canonical observations use
  existing shared admission/fallback rules. Canonical aliases come from the
  already compared evidence stage; dictionary overwrite and ordered output retain
  Python's last-row and sorted-key behavior.

## Verification

Tooling uses `mise exec rust@1.99.1 uv@latest -- ...`,
`MATURIN_NO_INSTALL_RUST=1`, `uv sync --locked --python 3.11`, and offline Cargo
(including child builds through `CARGO_NET_OFFLINE=true`). Environments, books,
recordings and mutation scratch trees use disposable `/tmp` roots.

- Formatting and workspace/all-target Clippy with warnings denied pass.
  `cargo test --workspace --exclude kassiber-net --locked --offline` passes
  **719 tests**. All **681 mutations** have named-test kills; the 39 new rules
  also pass the final full rerun, and whole-catalog applicability passes.
- Inventory/contracts/frozen and mutation-checker contracts pass **15 tests
  and 14 subtests**. The shared JSON-feature check reports **145 byte-identical
  ledger outputs**.
- The generator/partial-seam modules pass **13 tests**, including the complete
  2,000-book composition corpus, 24 permutations and 24 full-Python claim/audit
  controls. All eight graph scenarios retain 250 Python and 250 Rust successes.
  Together with the component corpus, the seven generated suites compare
  **14,000 seeded books**. Configured main permutations are evidence 58,
  components 2,000, Samourai 68, recorded pairs 92, fan-out 94, graphs 16 and
  composition 24.
- The custody Python modules plus ownership/transfers/reviews, historical
  compatibility and onchain modules report **773 passed and 2,679 subtests
  passed, with four failures**. Census compares components, evidence and all
  eight partial projections on **214 successful snapshots** without a
  difference. `interpretation` remains skipped on all 215 observed builds.
- The failures are the two `CustodyComponentDaemonSurfaceTests` daemon tests
  and the two real-JSONL `CustodyGapDaemonProtocolAcceptanceTests`, as at the
  consolidation checkpoint. A fresh disposable probe confirms the daemon's
  `internal_error`: the stable owner-lock runtime `~/.kassiber/run` is read-only
  in this sandbox. No owner guard is weakened. The orchestrator needs a writable
  runtime for these tests.
- The fresh recording set passes integrity and Rust replay: **206 requests and
  206 permutations**, zero pre-request refusals and zero differences. The earlier
  206-request corpus also passed replay with permutations before the final runs.

Full census, custody-shadow shards and the full quality gate belong to the
orchestrator; this checkpoint does not claim them. No push or PR is made.

## Remaining slices

1. Port raw-observation holds/privacy, channel lifecycle and loans, native HTLC
   transition production and reservations, native component reconciliation and
   final interpreter composition. Keep `interpretation` skipped until the entire
   stage compares successfully.
2. Port gap search, quantity arbitration and finalized projection before joining
   custody with `ledger`.
