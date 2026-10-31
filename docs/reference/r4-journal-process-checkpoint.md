# R4-13 journal process checkpoint

This is a **test-only, unregistered checkpoint**. The original
`t3code/r4-13-journals-process` starts at `b5eb38ea8` with loader rounds 3–8
(`0b78605a0..c5966c1ff`) integrated; the continuation
`t3code/r4-13-journals-process-2` starts at `925800ebe`. It does not complete R4-13 or switch
production authority. CLI `journals process`, `--auto-pair`, daemon
`ui.journals.process`, desktop refresh and AI mutations retain their existing
routing.

The conversion boundary also incorporates t3code/perf-conversions@60b1a4506.

## Continuation stage snapshot

The local continuation `t3code/r4-13-journals-process-2` starts at
`925800ebe`. Its initial handoff snapshot is:

| Stage branch | Committed tip inspected | Integration |
| --- | --- | --- |
| `t3code/r4-9-loader-9` | `c5966c1ff5bbb1bbad1a4b073adf2d10f468dc81` | Already inherited through loader-8; `loader-8..loader-9` is empty. |
| `t3code/r4-10-prebuild`, `-2` and `-3` | `bc9e126d3739b15945ba67542cdc5e30bd09abab`, `4e2ae2f0b`, `9ce29bfde33c9da35131e5577ff32fe64b6a204e`, `9fa4899afad00b500d2cedbb7c136f7cd550cc32` | Cherry-picked pricing, source repair/warning, legacy refresh/profile reload and the ordered/split APIs. `process_native` now uses the real repair/price APIs. |
| `t3code/r4-11-render-3` | `197f6cdecc2a8ed12f1ee78a32ef92264bc15c25` | Cherry-picked boundary hardening, preserving newer loader representation and process mutation recipes. |
| `t3code/r4-12-persist-2` | `b5eb38ea8cb764675322e61e9b957a57deb947c6` | Already an ancestor; no new committed persistence changes. |

The pricing cherry-pick retains the continuation's newer shared-column loader,
admission tests, process example and checkpoint documentation. Its older
loader build fixes do not replace loader rounds 3–8. Pricing adds no dependencies;
legacy refresh adds only the existing internal transfer crate and its lockfile
edge, with no new third-party package. Cached pricing alone does not implement
either repair. The later handoffs supply both repairs, profile reload and the
combined/split interfaces; the process adapter uses their returned profile.

The subsequently requested R4-14 handoff
`c4745075705ced90febdbb02ab0eab9684607eb8` is also cherry-picked. Its
`kassiber_transfers::pairing::auto_pair_before_journals` leaves successful
writes pending for the journal caller. Integration adapts transfer authoring's
snapshot constructor to the inherited shared-column `Record` representation
and private row cache; it does not revert the loader representation. The
[pairing handoff](rust-pairing.md#callers-and-integration) remains the contract
for the refresh coordinator; importing the stage does not register a route.

The renderer continuation adds 101 frozen cases and a separate native finish
entry. Its actual surrogate scalar and malformed/lossless boundary gaps remain
open, as recorded in the [handoff](r4-journal-render-boundary-checkpoint.md).
Inherited rendering campaigns are not full process/persistence verification.

## Continuation verification and routing prerequisites

The continuation imports pricing as `c0f8f4dcd`, pairing as `658e91260`, and
renderer hardening as `664d867bc`. The fresh integrated all-feature native run
passes **677 tests** across custody and journals; workspace formatting passes.
The R4-14 service run passes all **seven tests**, including full SQLCipher
service/dump parity and caller-savepoint rollback of both repeated passes.
After the later source-repair handoff, all **67 journal tests** pass again,
including its nine new source tests. Frozen source regeneration matches all
**49 complete dumps/refusals**. The fresh real-driver replay passes all
**204 source/pricing comparisons** (49 source plus 53 pricing cases on each
driver), including serialized source-field bytes. Its native socket syscall
guard and guard self-test pass; no performance benchmark is run.

The source-repair merged-state mutation campaign kills **57/57 recipes**, with all
**20 named unmutated baselines** passing and zero build failures. It includes
eight `JPROCESS` recipes, all 46 source/pricing recipes, both reanchored loader
admission/precedence recipes, and `GAP-120`. Every recipe is killed by every
named test declared in its catalog entry. The earlier 26-recipe run belongs
to the pricing-only boundary; the final run repeats those checks after source
integration and includes the reanchored shared-savepoint mutation. Source
hashes, per-recipe named tests and results are retained in
`target/r4-13/final-integration-mutation-summary.json`; both disposable mutation
build trees and scratch source copies are removed after summarizing them.
These bounded integration checks do not certify the full inherited catalogs
or future CLI/daemon/refresh wiring.

The earlier test-only process adapter optionally runs real native cached pricing.
Its `cached_pricing: true` mode requires `prepared_repairs_noop: true`; it never
supplies production resolution or either missing repair. The original
`prepared_noop: true` benchmark interface remains available. Live fixtures
check that Python's real resolution and repairs perform no writes before
allowing that prepared seam. The expanded process suite has 24 successful
comparisons (both codecs, generic/Austrian profiles, unchanged/changed/blocked/
empty/cached-price/legacy-price) and 12 refusal/recovery comparisons. Recovery
kills the native writer after writes 1, 8 and 15 on SQLite and SQLCipher,
reopens the book, checks integrity, and compares every stored scalar with the
starting book. Fault injection and pause controls exist only in the unpackaged
example; every paused child is killed and reaped by its supervising test.

The fresh focused live census passes **250 tests**, including all 36 process
cases. It records 76 exact comparisons for each of components, evidence,
interpretation, gaps, quantity, finalized projection and fusion (zero skipped),
147 native-loader v1 byte matches, and 270 production tax calls. Frozen pricing
regeneration matches all **53 complete dumps/refusals**. The frozen renderer
regeneration and persistence replay are included in that live run. Logs are
incremental under `target/r4-13/logs/`; these are focused fixture checks, not
the 24,000-book generated or 159-recording full-pipeline gates.

The post-source focused shadow run passes **38 tests**: the full 36-case
process matrix plus the existing daemon main-thread and consent/book-change
regressions. All 62 custody snapshots match frozen Python on all seven stages
(zero skipped), 62 native-loader snapshots match v1 bytes, and all 62 tax calls
match the oracle. The tax v2 selector reports 36 generic and 22 Austrian routes,
plus two not-routed calls for each; these reference-harness counters do not
certify CLI hybrid fallback behavior. The frozen generic/Austrian engine,
ledger and text suites pass all **14 test groups** with the pipeline's JSON
features. One existing `ezodf` invalid-escape deprecation warning is emitted;
there is no test failure.

All-target, all-feature clippy passes for journals and transfers with warnings
denied; workspace formatting passes. The required classifier update and CI
preflight pass (**57 tests**). Review descriptions now distinguish live pricing
parity, refusal rollback and abrupt recovery instead of incorrectly describing
all R4-10 work as a no-op. Manual second review finds no outstanding issue in
this unregistered checkpoint. `target/r4-13/verification-summary.json` binds
handoff SHAs, binary/log hashes and check counts; verification build trees are
removed after summarizing them. The source-repair boundary inspected loader
`c5966c1ff`, pre-build `4e2ae2f0b`, renderer `197f6cdec` and persistence
`b5eb38ea8`. No additional committed legacy-refresh handoff was available.

The persistence worker subsequently committed `32767d331`, which is imported
as a separate verified boundary. Its typed calculation encoder and borrowed
posting bindings preserve the storage format; relation fallbacks now reuse the
custody principal primitive and Python's first-truthy source identifier. It also
adds deterministic termination after every dirty-page DML and before/after
COMMIT on both codecs. Those stage checks complement the six process-level
termination cases above. The merge retains all process/renderer mutation entries
and the newer shared-column loader; none of the older restack loader edits is
applied. Performance remains a separate track, and this import does not change
production registration or close full-process campaign acceptance.
The merged persistence boundary passes all **85 native journal tests** and
**38 live Python process/storage tests** under custody/tax census: 76 exact
comparisons on each custody stage, zero skipped, and 76 production tax calls.
The source-boundary mutation results above predate this persistence import;
the affected persistence/process recipes must be rerun on the subsequent
pre-build integration.

The complete pre-build handoffs `9ce29bfde` and `9fa4899af` subsequently supply
legacy refresh/profile reload and the combined/split APIs. The continuation
imports the legacy stage as `16c145721`. `process_native` composes the real
split API with loading, building and persistence; the old prepared adapter is
retained only for explicit benchmark/fault probes. The transfer regression
suite passes **44 tests**, including the existing SQLCipher pairing and
outer-journal rollback checks. No third-party dependency is added.

The new process oracle freezes **11 full process books**: all eight ordered
pre-build fixtures, a genuine full-build refusal, a custody filed-impact case
and a chain retraction. Durable UUIDs and every repair/store/resolution clock
are injected before execution and compared without remapping. Python's random
`UUID.hex` savepoint names are transient and are excluded from the durable tape;
their generated values do not appear in any book or result. A precise build
clock is injected independently. Every stored scalar, rowid and serialized
field remains part of the comparison. Physical replays cover both codecs.

These fixtures exposed an incorrect `custody_quantity.blocked` response flag:
the serialized quantity state has no `report_blocked` property. The process now
uses `bool(issues)`, matching the Python runtime property. They also pin distinct
store, custody-impact-service and per-chain-resolution clocks, including the
custody service's clock read when its pending set is empty. The fixed-clock
persistence API remains compatible; the native process uses its new callback
variant. The imported pre-build build-refusal witness now supplies a valid
wallet and expects the renderer continuation's exact missing-row `TypeError`,
so its only fault remains inside actual building rather than wallet lookup.

The real-prebuild merged state passes all **102 native journal tests**, including
the combined API's six tests, four complete native-process tests, and the
persistence crash campaign. Python regeneration matches all **11 complete
process books**. Its focused custody/tax census passes **53 tests**: the full
43-case process suite, persistence/render oracles and both named daemon/AI
regressions. Each custody stage has **76 exact comparisons and nine skipped
patched-compute builds**; the tax census records 279 production calls. These
are focused integration checks, not the full generated/recorded campaigns.

The legacy mutation run exposed a missing repeated-drift witness. Two new
complete Python dumps cover a third immutable revision on a connected group
and the reference's lineage-exists refusal on a second single-pair drift. The
updated legacy suite passes all **eight tests** over **40 frozen books**. The
single-pair refusal is reproduced by
`uv run --locked python -m tests.rust_port.repro_journal_repeated_legacy_drift`:
choosing the sorted linked component as revision parent can supersede the
active revision before the later active-parent lookup. Python product code is
unchanged; native behavior preserves that refusal and its durable issue.

The integrated pre-build driver replay passes **300 complete comparisons**:
53 pricing, 49 source, 40 legacy and eight combined cases on each codec. The
native network syscall guard and its socket self-test pass. The selected
integration campaign kills **106/106 recipes** across persistence (59), the
original process (eight), the real process adapter (11), legacy refresh (21)
and the combined API (seven). All **34 distinct named baseline filters** pass;
there are zero build failures or remaining survivors. The first campaign
killed 105 recipes and exposed `JL-retarget`; after adding both second-drift
witnesses, the unchanged mutation is killed by both the full-dump test and
`prebuild_legacy_repeated_drift_retargets_each_immutable_revision`. Per-recipe
named tests, source hashes and both campaign logs are bound in
`target/r4-13/native-prebuild-final-mutation-summary.json`. Disposable mutation
trees are removed by the runner. This is a selected integration campaign,
not the full inherited catalog or future route/refresh wiring.

The final focused shadow run passes **45 tests**, including the full process
suite and both named daemon/AI regressions: **62 exact custody comparisons per
stage**, nine skipped patched-compute builds, **71 native-loader v1 byte
matches**, and **71 tax-oracle matches**. The tax reference harness's generic
and Austrian route counters do not certify hybrid command routing. All-target,
all-feature clippy for journals/transfers passes with warnings denied;
workspace formatting, all **nine network/process chokepoint tests**, and all
**14 frozen tax engine/ledger/text groups** pass. The required classifier
update and CI preflight pass (**57 tests**). Current and historical evidence
are separated in `target/r4-13/verification-summary.json`.

Production registration still requires the following concrete work:

- Bind `process_native`'s resolver callback to the shared production scope
  resolver under its reserved writer, and use `SystemSources` in product
  transports. The adapter runs source repair, warning construction, legacy
  refresh/profile reload, then pricing. Native transports must use
  `prebuild::source_overlap_result_fields` to preserve the nested Python field
  order; serializing the serde result dictionary directly loses that order.
  The private probe still injects the resolved profile and sources for exact
  comparison; it is not a production scope resolver or route.
- Finish the lossless stage boundary and exact refusal handling recorded by
  R4-9/R4-11. In particular, `process::stage_value` still explicitly refuses
  NaN, actual surrogate scalars and non-string keys; importing the renderer
  continuation does not remove those refusals.
- Implement Python's `refresh_journals_step` around the real R4-14 entry. A
  stale projection rebuild commits before pairing. Successful pairing leaves
  writes pending; the second rebuild must commit them together or roll them
  all back on failure/cancellation. Preserve skipped-pairing summaries,
  candidate-cache commits, `rebuilt`, skip-current behavior and phase order.
  The current plain native process rejects pending caller transactions and
  cannot simply be called after pairing without reconciling ownership.
- Register CLI plain and `--auto-pair` only after the integrated full-pipeline
  gates, then prove exact envelopes and zero Python fallback. Pairing is now
  available natively; keeping the flag on Python is no longer a scope rule.
- Register desktop processing with pairing and the internal AI mutation with
  pairing disabled. The latter has **no separate public route**: both use the
  name `ui.journals.process`, but AI dispatch runs inside `ai.chat` via
  `_execute_mutating_ai_tool` and `_run_scoped_ai_mutation`. Preserve its
  consent, input-version/book binding, main-thread execution and cancellation.
  Named reference tests include
  `DaemonSmokeTest::test_journal_process_tool_uses_daemon_main_thread_connection`
  and `DaemonSmokeTest::test_ai_mutation_rejects_book_changed_while_waiting_for_consent`.

The inherited verification, mutation and performance sections below describe
the original checkpoint. Continuation evidence is reported separately; none
of the inherited generated/rendering campaign results is a complete native
process/persistence campaign.

## Native composition

`crates/journals/src/process.rs` composes the native loader, shared custody
stages, fusion request, tax engine, renderer and persistence service. It is
available only with `test-harness` (or crate unit tests). The prepared-book
example is an oracle probe, never a packaged command.

The mandatory `Prebuild` trait is the R4-10 integration seam: scope/profile
resolution, repairs with profile refresh, and cached auto-pricing.
`MissingPrebuild` refuses explicitly. There is no production no-op default.
`process_native` implements this seam using the real split R4-10 API, an explicit
decimal context, a caller-supplied scope resolver and transfer `Sources`. It
samples the precise build clock after repairs/pricing and the row clock when
storing. `store_ledger_state_with_clock` samples each filed-chain resolution's
own timestamp. The older fixed-clock persistence entry remains compatible.
The example accepts `native_prebuild: true` with profile/clock/UUID tapes and
restricts physical database paths to this worktree's `target/`. Its explicit
`prepared_noop` and cached-price/no-op-repair modes remain only for inherited
benchmark probes. The live process suite now uses real pre-build throughout.

The caller-owned connection acquires `BEGIN IMMEDIATE` before profile reads.
Tax-policy admission precedes repairs. Repairs, pricing, build and persistence
run in the same transaction and savepoint. An RAII guard rolls back pre-commit
refusals and panics; persistence failures also roll back repairs and freshness changes.
Nested transactions are rejected before emitting progress. Scoped cache and
automatic checkpoint settings restore on exit. Progress follows Python's
non-pairing sequence:

`writer_wait → preparing → repairing → pricing → building → storing → complete`

COMMIT precedes `complete` and the returned success. `Committed::after_reply`
runs the deferred passive WAL checkpoint after the transport publishes and
flushes the response. SQLCipher retains DELETE/FULL. The example flushes before
consuming this handle. CLI and daemon transports are not implemented here.

The composition reuses the kernel stage representations and the existing
persistence boundary. The conversion checkpoint adds `ledger_typed` and passes
its native response directly to rendering. The renderer receives borrowed
`RenderOperands`, avoiding the aggregate JSON tree and its repeated stage,
row and ownership-index clones. Only native reconciliations are projected from
the interpretation stage. An in-memory request decoder still converts the
fusion builder's JSON value into `LedgerRequest`; quantity and finalized
projection values still use `stage_value`. Text is used at the engine boundary
only when that decoder refuses, retaining the original request's diagnostic
column. This remains a partial R4-8 handoff and does **not** satisfy the design's
no-JSON production-path gate.
Floating NaN, surrogate strings and non-string stage keys at the renderer seam refuse
explicitly pending lossless R4-11 integration. Other loader legacy-value and
refusal limitations remain those in the loader checkpoint.

## Verification scope

This section records the original integration's evidence. Fresh results for the
conversion boundary checkpoint appear below.

Six native integration tests cover fourteen frozen raw starting books with
complete Python dump expectations, ordered phases, writer reservation,
monotonic processing markers, repair/pricing participation, policy and fusion
refusals, nested-transaction rejection, persistence fault injection and panic
rollback. Every table, rowid, UUID and timestamp is compared. Frozen dump
comparisons preserve scalar types and REAL bits through the shared oracle.

Sixteen live Python comparisons cover SQLite and SQLCipher, generic and
Austrian profiles, unchanged/changed/blocked/empty books. They compare the
complete payload, ordered progress phases and all database tables without
masking. These are core payload/progress comparisons, not CLI-envelope or
daemon-event-stream acceptance.

The final affected Rust run passes **668 tests** across custody and journals,
with `test-harness` enabled and one test thread. Both crates pass all-target
clippy with `test-harness,timing` and warnings denied. Workspace formatting
passes. The final live loader/render/persistence/process Python run passes
**230 tests**, including all sixteen new pipeline comparisons. Verification
uses Rust 1.99.1, the locked Python 3.11 environment,
offline Cargo, eight build jobs and disposable roots under `target/`.
The quality gate and CI build both oracle examples, expose their paths to the
Python lane, and enable the native process integration tests explicitly.
The required classification update and CI preflight pass (**57 tests**).
Regeneration includes inherited disposition evidence and summary drift. A
documentation-only flag reorder makes the quantity checkpoint's offline
`uv sync --locked` example satisfy the locked-workflow check.

At the original integration, the 24,000 generated books with per-scenario
activity, 1,030 permutations, 159 retained requests and their permutations/fresh
recordings, census and shadow campaigns had **not** been rerun. Sampled and
inherited component evidence cannot substitute for those gates. No served shape
or zero-hybrid-fallback result is claimed.

## Mutation ownership

The orchestration catalog adds the following deliberately broken wiring,
each assigned a named test. The catalog remains partial rather than an
exhaustive branch inventory.

| Mutation | Required killing test |
| --- | --- |
| `JPROCESS-writer-lock`, `JPROCESS-commit`, `JPROCESS-all-counters`, `JPROCESS-stale-marker` | `process_complete_dumps_match_python_from_raw_books` |
| `JPROCESS-outer-rollback` | `process_prebuild_refusals_and_phase_panics_restore_the_entire_book` |
| `JPROCESS-repair`, `JPROCESS-pricing` | `process_commits_repairs_and_pricing_with_the_projection` |
| `JPROCESS-tax-before-repair` | `process_refuses_tax_policy_before_prebuild_and_rolls_back_fusion_refusals` |

Loader integration moved `JL-receipt-accounting-precedence` to typed
admission and `JL-projection-loaded-admission` into the shared native runner.
`GAP-120` now targets the shared-column worklist adapter. These retain their
original rules and named tests. None is removed or weakened. The full inherited
catalog still requires a fresh run before acceptance.

All **11/11 selected mutations were freshly killed**, with seven named tests
passing first on the unmutated disposable tree. The scratch tree and its
1.6 GiB build were removed after recording the result. Applicability checks
pass for all 120 journal, 136 loader and 2,327 custody catalog entries; these
counts are separate catalogs, not a fresh full-catalog kill claim.

## Performance probe

`scripts/benchmark-journal-process.py` runs five alternating Python/native
pairs on disposable copies of the four committed-seed benchmark books. It
compares payloads, phase order and complete scalar-exact dumps outside the
timed subprocess. Process CPU includes all native worker threads, response,
deallocation, deferred checkpoint and exit. Internal phase timestamps stop at
the reply boundary; opt-in custody spans measure wall time and may overlap.

The prepared probe excludes actual native R4-10 resolution/repair/pricing and
the registered CLI/daemon shell. Its numbers cannot close the end-to-end
authority or ratio gates. Shared-host load is recorded; CPU is the primary
comparison under the owner's shared-machine rule. Wall times remain recorded
but do not meet the design's idle-host measurement condition.

All **20/20 paired complete dumps**, payloads and phase sequences match. The
[raw measurements](r4-journal-process-prepared-measurements.json) retain every
trial, generator metadata, phase timestamp, wall span and binary hash. Each
timing cell below is **best / median of five**, in seconds. The design targets
refer to the registered command's wall time; this probe only diagnoses the gap.

| Book | Python CPU | Native CPU | Native wall | Median CPU speedup | Design end-to-end target |
| --- | ---: | ---: | ---: | ---: | ---: |
| generic 2k | 2.114 / 2.179 | 1.730 / 1.784 | 1.396 / 1.441 | 1.22× | ≤ 0.45 s |
| multi-wallet 2k | 2.519 / 3.107 | 2.368 / 2.635 | 1.834 / 2.196 | 1.18× | ≤ 0.60 s |
| generic 20k | 16.646 / 18.911 | 16.737 / 20.132 | 13.147 / 15.997 | 0.94× | ≤ 3.2 s |
| multi-wallet 20k | 26.909 / 27.481 | 21.566 / 24.178 | 16.733 / 18.838 | 1.14× | ≤ 5.2 s |

The owner's **4× target is unmet on all four books**. Native median CPU would
need to fall by approximately 69%, 71%, 77% and 72%, respectively, relative to
these same-run Python measurements. The 1-minute load ranges were 75.9–89.6,
63.4–72.4, 43.3–57.5 and 41.7–48.4. This is evidence under shared-host load.
Idle-host performance acceptance remains open.

Native median CPU between `building` and `storing` is 1.468 / 2.072 / 14.819 /
19.824 s in book order; persistence through COMMIT is 0.158 / 0.231 / 2.285 /
2.371 s. Thus the build and its adapters dominate, rather than writes alone.
The generic/multi-wallet 20k wall-span medians expose the current composition
costs (spans can overlap and are not CPU attribution):

| Native span | generic 20k | multi-wallet 20k |
| --- | ---: | ---: |
| Snapshot load | 0.404 | 0.746 |
| Custody including typed admission | 4.956 | 7.158 |
| Renderer operand materialization and context | 3.425 | 3.143 |
| Fusion, engine text call and response conversion | 1.728 | 2.190 |
| Renderer | 0.239 | 0.278 |
| Persistence and COMMIT | 2.394 | 2.984 |

The no-op pre-build cannot account for this shortfall. Remaining native-to-JSON
conversions, cloned stage operands, custody work and post-COMMIT destruction
need profiling and integration work. Every line above that exceeds the design
by more than 10% remains an open performance defect; no budget exception or
authority switch is claimed.

## Conversion boundary checkpoint (partial R4-8)

The native process now calls `ledger_typed` and gives its `LedgerResponse`
directly to rendering. An unrepresentable request alone takes the original
text boundary, so overflowing amounts keep the exact request-column refusal.
The JSON renderer adapter remains available to the oracle; native composition
constructs borrowed `RenderOperands` and projects only the interpretation's
native reconciliations. Custody kernels and persistence implementation are
unchanged.

This is a bounded checkpoint, not completion of the requested typed handoff.
Fusion still builds a JSON value, decoded in memory into `LedgerRequest`.
`stage_value` still materializes quantity and finalized projection values.
Replace those three adapters before accepting R4-8 or the no-JSON gate.

### Before/after process CPU

The baseline is `925800ebe` with timing instrumentation alone. The
[raw record](r4-journal-conversions-measurements.json) embeds the exact baseline
patch, source and binary hashes, all samples, dump hashes, load averages and
CPU/wall spans. Both binaries use Rust 1.99.1 release builds, offline Cargo and
eight build jobs. Three books were regenerated from the committed generators;
the multi-wallet 20k book reuses a checkpointed disposable loader fixture after
checking every CSV hash and its 300 authored pegs.

Five adjacent before/after pairs alternate AB/BA; each pair also runs Python
on another disposable copy. All **20/20 three-way comparisons** match complete
scalar-exact dumps, payloads and phase order. There are zero engine text-refusal
spans in the twenty successful candidate processes. The 1-minute load ranged
from **40.01 to 66.67**. These remain prepared-book measurements on a shared
host; R4-10 and registered command acceptance remain open.

CPU uses `getrusage(RUSAGE_SELF)` and includes every worker thread. Timing's
CPU clock is injected by the example, with no new dependency or product I/O;
without the `timing` feature these spans are disabled. Whole-process CPU also
includes response, destruction, deferred checkpoint and exit. Each cell below
is **best / median of five**, in seconds.

| Book | Before CPU | After CPU | Python CPU |
| --- | ---: | ---: | ---: |
| generic 2k | 1.753 / 1.905 | 1.285 / 1.602 | 2.089 / 2.635 |
| multi-wallet 2k | 1.940 / 2.386 | 1.618 / 1.964 | 2.431 / 2.596 |
| generic 20k | 16.685 / 20.446 | 13.539 / 16.155 | 15.865 / 18.635 |
| multi-wallet 20k | 20.492 / 27.492 | 17.133 / 25.083 | 29.732 / 35.873 |

The following process-CPU spans come from each version's fastest complete
process. They are inclusive: `fusion_engine` contains request and engine spans,
and the candidate engine boundary contains request conversion and computation.
Do not add overlapping spans. Every trial's spans and wall times remain in the
raw record.

| Span, before → after CPU seconds | generic 20k | multi-wallet 20k |
| --- | ---: | ---: |
| Snapshot load | 0.384 → 0.458 | 0.491 → 0.540 |
| Custody including typed admission | 7.517 → 8.923 | 11.417 → 10.339 |
| Renderer operands and context | 3.205 → 0.822 | 2.449 → 1.009 |
| Fusion request value construction | 0.511 → 0.355 | 0.540 → 0.456 |
| Engine boundary including conversions | 1.324 → 0.443 | 1.806 → 1.074 |
| Fusion and engine together | 1.835 → 0.798 | 2.346 → 1.530 |
| Rendering and joins | 0.218 → 0.168 | 0.282 → 0.229 |
| Persistence and COMMIT | 1.371 → 1.062 | 1.815 → 1.660 |

Candidate request conversion adds **0.066 / 0.094 s** inside the engine
boundary; typed engine computation is **0.321 / 0.907 s**. Request construction
plus conversion is **0.421 / 0.550 s**, still above the **0.10 s** target.
Whole-process best CPU improves **18.9% / 16.4%** against the same-run native
baseline. Its Python ratios are **1.17× / 1.74×**, below the owner's 4× target.
Unchanged custody and storage spans also vary under this load; their differences
are not credited as optimizations. Neither the remaining conversion targets
nor whole-pipeline performance acceptance is closed.

### Verification of this checkpoint

The affected Rust suites pass **823 tests**: 619 custody, 51 journals and 153
tax-core tests. Journals and tax-core frozen vectors include typed/text response
byte equivalence, refusal attributes, typed header admission and overflowing
amount diagnostic columns. All-target clippy passes with
`test-harness,timing` and warnings denied; workspace formatting passes.

The live loader/render/persistence/process and ledger Python suites pass
**1,145 tests and 14 subtests**, including all sixteen SQLite/SQLCipher
prepared-process comparisons. All twelve generated suites pass **24,000 books
and 1,030 permutations**, recording per-scenario outcomes and distinct active
seed requests. The retained corpus passes **159 requests and 159 permutations**
with no skips or differences. The comparison-only `journal-fusion` example
checks each emitted engine request against the text oracle: response bytes on
success and refusal attributes on failure. This covers the seven custody/fusion
stages and the engine boundary; it is not 24,000 complete database-dump
comparisons. Fresh census/shadow and registered-command acceptance remain open.

All **33/33 touched-file journal mutations and 189/189 ledger mutations were
freshly killed**, after 10 and 122 named baseline tests passed respectively.
The journal run includes every existing recipe for the changed process,
boundary and join files plus four new conversion-wiring recipes. The ledger
catalog adds the two typed-header recipes. Existing anchors still match exactly
once; none required relocation or weakening. Applicability checks cover all
124 journal, 136 loader, 2,327 custody and 189 ledger entries. The untouched
journal, loader and custody recipes are not claimed as freshly killed.

The [verification record](r4-journal-conversions-verification.json) retains the
commands, source and recording hashes, complete per-scenario activity, named
kill sets, logs and exact campaign harnesses. Verification uses offline Cargo,
eight build jobs, locked Python 3.11 and disposable roots under `target/`.
The required classification update passes without inventory changes; CI
preflight passes **57 tests**. The final workspace formatting check passes.

## Remaining integration work

1. Bind the real `process_native` adapter to production scope resolution and
   `SystemSources` through the shared book service. Source repair/warning,
   legacy refresh/profile reload, cached pricing and lifecycle clocks are now
   composed; preserve the ordered source fields when emitting envelopes.
2. Finish inherited R4-9/R4-11 acceptance gaps, including lossless stage values,
   exact refusals and the intended native boundaries. The committed handoffs
   listed above are imported; their open acceptance work remains open.
3. Add/run resumable complete-process shards for the twelve generators
   (24,000 books and 1,030 permutations), all 159 retained recordings and their
   permutations. Finish integrated frozen, census/shadow, codec/rollback and
   inherited mutation gates. The focused fixture checks above do not close
   those campaigns; future route/refresh wiring needs its own named mutations.
4. Register CLI `journals process` with and without `--auto-pair`, verify exact
   envelopes, routing and zero fallback, and commit that boundary. The native
   R4-14 stage is imported; its refresh coordinator still needs integration.
5. Register daemon processing only after reconciling the pairing contract and
   proving exact payload/event-stream parity, cancellation/scope and fallback
   behavior; commit separately. The current desktop kind accepts no arguments
   and calls `_journals_refresh_payload` with its default `auto_pair=True`.
   The scoped AI path explicitly passes `False`. A non-pairing replacement
   cannot silently stand in for both. R4-14 and the existing consent boundary
   must be preserved.
6. Keep performance acceptance on the separate worker track: measure the
   registered command from process start to exit against the owner's 4× target
   and four design budgets, then address R4-15/R4-16 authority work.
