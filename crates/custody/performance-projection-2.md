# Finalized projection borrowing checkpoint

Local branch `t3code/custody-perf-projection-2`, based on `238422f90`.
Changes stay in the finalized-projection modules, their boundary, tests and
mutation catalog. No dependencies, other custody stages or product routes change.

The compiler now retains borrowed source rows plus generated field overrides.
Principal and fee rows copy their retained payload directly into the existing
Python output once. Source rows remain immutable, explicit null overrides replace
source values, and output keys retain their previous order. Reviewed conversion
matching reads these rows through the same field interface and consumes existing
pairs rather than cloning another complete operand/result tree. The public
comparison helpers retain their existing JSON models.

Empty reviewed-conversion populations no longer build a row index. Empty barrier
populations no longer construct observation pools or materialize event-order
operands. Complete event operands still undergo exact-value admission, including
unused surrogate and nonfinite values. Quarantine finalization builds projected
anchor indexes only when held receipts need them. Machine-sized generated
integers avoid a decimal-text/JSON round trip; larger integers retain the exact
fallback. No new parallelism is introduced.

This is a kernel checkpoint. It does **not** complete R4-7's field-based compiler,
its public typed finalized projection, or the no-`Value` stage target. Input
conversion, generic selection, owned pair records and the existing Python output
boundary remain. All output conversion remains inside measured custody; no cost
is moved to fusion to manufacture a gain. The aggregate 10× Python target remains
open.

## Measurement

The inherited committed-seed generic and multi-wallet 20,000-transaction books
run through the prepared, unregistered R4-13 process harness. Native R4-10
pre-build preparation and registered CLI/daemon routing remain excluded. Each
comparison alternates five pairs, using fresh copies of the same migrated book.
The complete scalar-exact database dumps, reply payloads and phase sequences are
compared outside timing.

The clock is process user plus system CPU across all threads. A scratch-only
preload maps native `Instant` spans to `CLOCK_PROCESS_CPUTIME_ID`; Python uses
`process_time`. Each reported engine's stage values come from its fastest complete
process trial, with medians and all raw trials retained separately. Nested spans
overlap. Builds use the existing Rust 1.99.1 and locked Python 3.11, eight Cargo
build jobs, offline Cargo, and release debug information at level 1. Measurements
on this shared host do not establish idle-host performance acceptance.

The baseline task-clock sampler captured 9,009 generic and 11,737 multi-wallet
samples, with zero lost records. Allocation, JSON tree traversal, cloning and
destruction are visible. Incomplete frame-pointer callchains and unresolved
system-library frames prevent precise projection-only percentages. Separate
`finalized.select` and `finalized.emit` spans expose the candidate's selection
and complete output-emission costs.

Final measurements and verification are recorded in
[`performance-projection-2.json`](performance-projection-2.json).

## CPU results

Seconds from five alternating pairs per book, with all stages retained from each
engine's fastest complete-process trial:

| Stage | Generic before | Generic native | Multi-wallet before | Multi-wallet native |
| --- | ---: | ---: | ---: | ---: |
| Finalized projection | 1.0958 | 0.8306 | 1.1430 | 0.8637 |
| Custody total | 4.1625 | 3.7334 | 6.3003 | 5.8932 |
| Complete process | 11.3025 | 10.4814 | 14.2102 | 13.3168 |

Projection saves 24.2% CPU generic and 24.4% multi-wallet. Best-trial custody
improves by 1.115× and 1.069×. Median custody CPU is 4.3092/3.8508 s generic
and 6.3264/6.2492 s multi-wallet; the smaller multi-wallet median gain and the
two slower candidate process trials remain visible in the raw data. Median
projection CPU is 1.2197/0.8306 s generic and 1.1770/0.8921 s multi-wallet.
One-minute loads span 23.72–26.84 generic and 19.84–27.37 multi-wallet.

Independent Python/native pairs:

| Stage | Generic Python | Generic native | Multi-wallet Python | Multi-wallet native |
| --- | ---: | ---: | ---: | ---: |
| Finalized projection | 0.4759 | 0.7970 | 0.4590 | 0.8388 |
| Custody total | 5.5293 | 3.5850 | 12.4566 | 5.8516 |
| Complete process | 13.4565 | 9.8833 | 21.4731 | 13.0628 |

Custody speedups are 1.542× generic and 2.129× multi-wallet. Projection remains
slower than Python. Median custody CPU is 5.6066/3.7391 s generic and
14.0855/5.9429 s multi-wallet; median projection CPU is 0.4785/0.8185 s and
0.5251/0.8639 s. One-minute loads span 18.36–19.90 and 18.06–21.62.
All twenty pair comparisons agree on complete scalar-exact database dumps,
reply payloads and phase sequences.

In the Python/native trials, native projection input, selection and output
emission take 0.1871/0.3524/0.1791 s generic and 0.1917/0.3736/0.1732 s
multi-wallet. These nested spans do not cover all projection cleanup and are not
independent stage budgets. The complete native custody totals still exceed the
0.51/1.21 s budgets by 7.03×/4.84×. Finishing a field-based compiler and removing
the remaining operand/output materialization are open work; this checkpoint
does not establish R4-7 or 10× custody acceptance.

## Verification

Formatting, clippy with warnings denied and all 629 Rust custody tests,
including frozen vectors, pass. All 24,000 generated books and 1,030
permutations pass: 50,060 byte comparisons and 178 matching refusals. All 159
recordings and 159 permutations pass, with 636 byte comparisons. Per-scenario
activity and refusal signatures remain in the machine-readable record.

All 238 touched-file recipes, including the ten additions, are freshly killed
under the final source/catalog fingerprint. Each mutant must compile with
offline `cargo check --locked --lib --tests` and produce positive failing-test
counts; a compilation error is not a kill. The whole catalog passes:
**2,344/2,344 recipes killed**, with no survivors or build/runner errors.
The eight serial, resumable shards select 293 recipes each in the first six
shards, then 270 and 78, skipping the 238 already confirmed kills. Every shard's
unmutated baseline passes. The final source and catalog fingerprints match the
completed mutation state.

The required `classify_tests update` passes without an inventory disposition
change. The required CI preflight passes all **57 tests**. The final evidence
writer independently recomputes the source fingerprint and verifies complete
catalog coverage after those checks.

## Mutation accounting

The catalog retains every inherited recipe, rule and test assignment. Twelve
recipes move to the equivalent borrowed-field access or generated fee assignment;
ten new recipes cover overlay emission and the empty-population shortcuts. The
catalog grows from 2,334 to 2,344 entries.

The first new-recipe run killed nine mutants and exposed one test-assignment gap:
`projection-empty-barriers` pointed at scoped-basis vectors, all of which contain
a barrier. The existing selected-composition vectors exercise the absent-barrier
path and kill it. Production code is unchanged. The earlier nine kills retain
their original source/catalog fingerprints; mutation accounting restarts under
the corrected catalog, where all ten new recipes are freshly killed.

Disposable mutation and verification build/source trees and scratch book copies
are removed after summarizing their results. Saved benchmark executables, input
books, raw paired trials, profiles, logs and mutation state remain under
`target/custody-perf-4/` and `target/projection-2/`. An empty temporary directory
remains for the local helper's `TMPDIR`. No test or background job remains running.
