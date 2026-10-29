# R2e quantity independent-review corrections

Local branch `t3code/r2e-quantity-fixes`, based on `9d2ab49c`. No projection
implementation, production Python accounting, request schema, dependency or
network behavior changes. No push or pull request.

## Local implementation commits

| Commit | Scope |
| --- | --- |
| `2951cdc8` | Late stage refusal comparison and inventory regressions |
| `122f64d6` | Exact component arithmetic and wide allocation regressions |
| `618ae9f3` | Quantity seam parity, operator sweep and named witnesses |
| `a89ce4fd` | Explicitly scoped injected stage failures |
| `0e1e270b` | Independent input counts, raw activity and terminating FIFO witnesses |

## Findings and named witnesses

| Finding | Correction | Named witness |
| --- | --- | --- |
| P2-1 late Python refusal escapes comparison | Compare after Python's outcome, preserve matching exceptions, validate complete stage inventories | `test_quantity_refusal_compared_after_gaps`, `test_rust_quantity_refusal_against_python_success`, `test_session_rejects_unaccounted_quantity_stage` |
| Intentionally patched recorder stage | Scoped annotation for the exact stage and exception identity; explicit injected skip reason and inventory | `test_injected_failure_requires_exact_stage_and_exception`, `test_session_rejects_invalid_injected_skip`, `test_recorder_failure_has_explicit_injected_skip` |
| Rejected timestamp minimum | Separate rejected-row timestamps | `quantity_rejected_rows_earliest` |
| Strictly positive blocker amount | Zero remains unquantified | `quantity_zero_blocker_unquantified` |
| Claim bundle anchor selection | Observation and persistence anchor differ | `quantity_bundle_anchor_identity` |
| Reconciled component filter | Freeze a recovered native route | `quantity_native_reconciled_filter` |
| Evidence subject selection | Freeze active authored multi-allocation evidence whose absence changes runtime composition | `quantity_evidence_subject_identity` |
| P3-2 companion activity inflation | Count only each raw seed's decisions, issues or refusal; retain empty parity controls separately | `test_every_quantity_raw_generator_matches_both_engines`, `test_vacuous_quantity_scenario_fails_activity_check` |
| P3-2 duplicate variants and calls | Deduplicate variants and both engines' operand evaluations across scenario labels | `test_generated_quantity_variants_are_distinct`, `test_quantity_duplicate_operands_are_not_counted_as_calls`, `test_quantity_python_duplicate_operands_are_evaluated_once` |
| P3-3 unknown decision state | Admit only the four finalized states | `quantity_unknown_state_ineligible` |
| P3-3 empty gap asset | Refuse at identity validation, preserving Python's message and empty compiler details | `quantity_empty_asset_identity_refusal` |
| P3-3 empty observation hash | Construct/refuse source and target slices in Python's order | `quantity_empty_observation_hash_refusal`, `quantity_empty_target_hash_message` |
| Pre-existing component integer panic | Exact arithmetic through conservation, allocation coverage, chronology, locations, anchor coverage and native support | `components_allocation_beyond_i128` |

The component integer witness includes unequal 90-digit allocations, wide leg
and valuation totals, suspense fee coverage, conversion valuation arithmetic and
native support. Python accepts these inputs and returns component diagnostics;
a machine-width refusal would therefore differ from Python.

The trusted review `survivors.py` inputs reproduced all three primitive
survivors. Its sole output artifact was redirected into `/tmp`; original review
inputs and binaries were preserved. The trusted fail-open harness now fails
with a recorded custody difference instead of silently passing.

## Independent generator counts

Counts below are distinct serialized private operands, not loop iterations,
scenario labels, claims, repeated companions or repeated successful results.
Both engine comparison lanes deduplicate before evaluating the operands.
Reversed operands are independently evaluated and deduplicated separately.

| Generator | Previous primary iterations | Distinct primaries | Distinct companions | Additional inputs from companions |
| --- | ---: | ---: | ---: | ---: |
| Runtime | 6,220 | 1,216 | 4 | 0 |
| Reviewed/native compilation | 3,660 | 660 | 2 | 0 |

| Runtime family | Distinct inputs |
| --- | ---: |
| Gap holds | 384 |
| Scoped tax barriers | 256 |
| Whole state | 252 |
| Component diagnostics | 324 |
| **Total** | **1,216** |

| Compiler family | Distinct inputs |
| --- | ---: |
| Reviewed | 324 |
| Native | 336 |
| **Total** | **660** |

The old “12,440 calls per engine” and “7,320 compiler calls” claims counted
repetitions and are withdrawn. The activity manifests retain per-scenario
labels but are not independent-input totals.

Exactly 14 raw scenarios remain empty over all 2,000 seeds: nine evidence
controls, `fanout.zero_source`, `safety.privacy_unknown`,
`channels.missing_funding`, `channels.loan_unknown_role`, and `gaps.large_empty`.
`custody_quantity_inactive_controls.json` lists the exact names. They remain
empty-result parity controls, preserving the inherited corpus, and are removed
from activity claims. Their companion never contributes activity to them.
All other scenarios must produce their own decisions, issues or refusal in
both engines. One additional raw migration refusal runs through the same
comparison loop and is required to have its own activity count.

A direct deliberately vacuous call exits 1 with
`AssertionError: vacuous quantity scenario`; its named regression catches that
failure. This verifies the activity assertion rather than merely observing
positive totals.

## Operator sweep and mutation accounting

The sweep includes comparison and min/max predicates across `runtime.rs`,
`holds.rs`, reviewed/native compilation, arbitration and FIFO allocation, plus
independent bundle timestamp and raw-wrapper field choices. The five reported
survivors have separate witnesses above. Additional witnesses distinguish
inclusive fee principal, invalid-observation message precedence, zero fallback
sources, zero unobserved allocations, equal-quantity direct payouts, and mixed
bundle priorities. An exact-capacity FIFO witness makes a nonterminating loop
mutation fail immediately at its exhausted boundary instead of counting a
hung trial as a kill.

One main-loop zero-allocation comparison mutation is excluded as equivalent:
explicit allocations with both amounts nonpositive are removed by flattening;
inferred source/sink rows are positive. Changing only that later zero boundary
cannot change an admitted compiler result. The earlier flattening zero boundary
has its own non-equivalent missing-location witness and is counted.

Thirteen inherited component recipes retain their original semantics with
`BigInt` operands and typed zero literals. Nonbuilding recipes, interrupted
trials, disk-quota failures, failed baselines, hangs and equivalent mutations
contribute no kill. Each counted mutation requires a passing unmutated named
witness, a building mutant and a failing named witness.

All **2,028 entries** were killed: **1,918 inherited and 110 additions**.
The local evidence bundle is `target/r2e-quantity-review/`;
`inherited-results.json` and `new-results.json` map every entry to its named
witness and successful kill log. The bundle also preserves generator, replay,
activity-falsification, workspace, clippy, focused overlay and trusted-review
reproduction logs, plus the bounded continuation script.

## Verification

| Check | Result |
| --- | --- |
| Inherited raw generators | 24,000 books and 1,030 permutations, zero differences |
| Additional raw refusal | Same generator comparison loop, matched `ValueError` |
| Retained replay | 159 requests and 159 permutations, zero differences and pre-request refusals |
| Activity falsification | Deliberately vacuous scenario exits 1; named regression passes |
| Distinct operand lanes | 1,216 runtime and 660 compiler inputs per engine; independent reversed evaluation |
| Rust workspace | 803 tests passed |
| Formatting and clippy | Workspace formatting and all-target clippy with `-D warnings` passed |
| Focused census | 80 tests and 21 subtests; 62 quantity comparisons, zero differences |
| Focused shadow | 78 tests and 21 subtests; 60 quantity comparisons, zero differences; documented read-budget deselection |
| Snapshot recorder module in census/shadow | 16 tests in each; one explicitly injected build in each |
| reports-contracts shadow | 1,690 passed, 146 subtests passed, 1 skipped; zero custody differences |
| reports-contracts census | 2,259 passed, 2,820 subtests passed, 1 skipped; zero custody differences |

The full shard commands use `mise exec rust@1.99.1 uv@latest --`,
`UV_OFFLINE=true`, `CARGO_NET_OFFLINE=true`, and
`MATURIN_NO_INSTALL_RUST=1`:

```sh
uv run --locked python scripts/python_test_shards.py run-custody-shadow reports-contracts --ci -q
KASSIBER_TEST_CUSTODY_ENGINE=census KASSIBER_TEST_TAX_ENGINE=census uv run --locked python scripts/python_test_shards.py run reports-contracts --ci -q
```

The first parallel mutation attempt exhausted the shared disk quota. Its
interrupted/error entries were discarded. The continuation uses separate,
bounded build targets, and builds and executes each mutation's named test
binaries. The ordinary sandbox workspace run encountered four denied test
socket binds; the complete run with disposable local binds permitted passed.
Locked shard setup initially lacked a build dependency in the temporary uv
cache; the requested runs were restarted using the existing cache with uv and
Cargo both offline.

No review finding is disputed. Keeping the fourteen empty cases as parity
controls, outside the activity inventory, preserves the requested inherited
24,000-book comparison while removing the inflated activity claim.
