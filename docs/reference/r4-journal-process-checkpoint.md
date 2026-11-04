# R4-13 journal process checkpoint

The earlier sections record a **test-only, unregistered checkpoint**. The
[fifth continuation](#fifth-continuation-production-binding) records production
binding and plain CLI registration separately. The original
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
explicitly pending lossless R4-11 integration. The accepted R4-9 loader has
no pending exceptions; its acceptance does not close this renderer seam.

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

> Inherited from `t3code/r4-13-journals-process-6@684c76bad`; combined R10 results are in the [R10 audit](rust-backend-r10-integration.md).
## Third continuation: merged stages and process campaigns

The local `t3code/r4-13-journals-process-3` continuation starts at
`98402b6b4`. It imports these newer stage handoffs before changing orchestration:

| Stage tip | Local integration |
| --- | --- |
| Loader `e15900677` | `9ac6c43ed` |
| Loader BLOB `d03c0b9a1` | `bb4fe55d8` |
| Accepted loader determinism `aaf6216e3` | `718c49640` |
| Pre-build `9e8979d30` | `0fafafe99` |
| Renderer `4656ef30e` | `388e9b492`, `9190704c9` |
| Persistence `51bd20498` | `3d3bbfe0c` |
| Pairing `c47450757` | Already inherited as `658e91260` |

All 76 loader vectors from loader-9 and all 66 from its BLOB continuation are
retained unchanged. The pre-build campaign, renderer, persistence and pairing
fixture files are byte-identical to the requested stage tips. Merge repairs
adapt the typed `JournalMsat` and shared source representation to persistence;
they do not regenerate expected dumps. The JSON snapshot adapter escapes
reserved scalar tag maps in its stage view, retaining the original typed wire
for v1 round trips. Native SQLite bytes retain their separate consuming policy.

`scripts/check-journal-process-campaign.py` executes the real native pre-build,
loader, custody, ledger, renderer and persistence through the unregistered
`journal-process` example. It compares outcomes, complete refusals, every
application-table scalar and rowid (including REAL bits), progress phases and
the complete durable UUID and lifecycle-clock tapes. Native socket syscalls
are denied; reference network entry points raise before any request.

The generator adapter imports physical stored columns and counts omitted
derived fields. It never infers ownership inventory, receipt authority or
legacy pairing links from a snapshot verdict. A malformed component witness
that cannot fit the current physical constraints is replayed with the same
explicit relaxed table schema in both runtimes; its original values and
duplicate ordinals remain intact. These are stored-book process comparisons,
not a claim that the reconstructed book recreates every original snapshot-only
fact. Fresh full-book recordings remain a separate gate.

Campaign output is confined to the worktree's `target/` directory. Mode-0600
JSONL shards append and synchronize each comparison; mode-0700 directories
also hold any complete private failure operands. A manifest binds the binary,
source, corpus and shard layout. A resumed run refuses changed evidence and
skips only already logged indices. Use the same output directory and shard
layout to resume; use a new directory after a source or binary change.

The initial generated run found a complete-refusal mismatch at operand 6,454:
native processing rolled back correctly but dropped the network-review hint
and details. Accounting admission now retains those attributes through the
loader's refusal-precedence path and the process adapter. The v1 custody error
document keeps its original three-field shape. Three added complete-process
books pin conflicted, mixed and binding-mismatch histories separately.

Focused merged verification also exposed two BLOB defects. The optimized
finalized-projection conversion treated private byte tags as ordinary maps;
it now uses the shared scalar conversion before moving ordinary containers.
Pre-build source reads rejected BLOB configuration before its consumers could
apply their existing policy; those reads now retain native byte identity.
Three additional complete-process books cover empty, binary and JSON-looking
wallet configuration. All eleven inherited complete-process records and their
base remain unchanged; the expanded oracle has seventeen books. No Python
product behavior was changed.

The fresh native custody/journal run passes **744 tests across 24 targets**.
The first focused Python run passed 316 tests and exposed the six BLOB
failures above; the repaired BLOB replay passes **51 tests**, including SQLite
and SQLCipher process comparisons. Formatting passes. Static applicability
passes for all **2,909 recipes** in the four affected catalogs; executable
mutation kills and complete campaign results are recorded separately below.

Before the later loader-determinism addendum, the fresh generated campaign
passes **25,030/25,030 operands**: **24,000 books**, **1,030 permutations** and
**1,207 exact refusals**. Every result, logical dump, phase and lifecycle tape
matches. Its binary SHA-256 is
`6baaeaa0aa6282bdca71574d844b0ae558214b7002c9d3eb3569afea1ee4a8f0`;
source fingerprint is
`30950407210014ec0020f9329335777af3bd8c51c98af3371c889e7a60227035`.
The 101 private shards and aggregate are under `target/r4-13/campaign-c/`;
progress is in `target/r4-13/logs/campaign-c.log`. This evidence belongs to
that source and binary, not the subsequent accepted loader integration.

The subsequent owner-requested `aaf6216e3` is integrated as `718c49640`.
The native loader and port-branch Python reference now reject mixed revision
families with code `validation` and message
`Custody revision references contain incompatible types`. All forty historical
loader exceptions are closed; the accepted loader has no pending exceptions.
Conflict resolution retains the newer process, pre-build, render and
persistence work and regenerates the combined test inventory. The accepted
loader integration again passes **744 native tests** and **57 preflight tests**.

## Atomic automatic-pairing refresh continuation

The unregistered `process::refresh_native` composes the real R4-14 entry with
the complete journal pipeline. A stale projection commits before pairing;
successful pairing and candidate-cache writes remain pending until the second
rebuild commits. A failed or cancelled second rebuild rolls back those pending
writes while retaining the first successful projection. No-pair reuse and
`skip_rebuild_when_current` preserve their reference results and lifecycle tapes.
The outer freshness callback runs only while the connection is idle, so a
progress observer cannot accidentally commit pending pairs.

One deferred WAL checkpoint spans both commits and remains deferred until the
transport publishes its response. Cache and checkpoint settings restore exactly
on success, refusal and panic. The ordinary process entry still refuses an
ambient caller transaction before emitting progress; only the private refresh
coordinator can join its reserved pairing transaction.

The committed-result handle now includes the Python insertion-ordered payload,
constructed before COMMIT. It preserves nested quantity counters, warnings,
component blockers and source repair/warning fields. The test probe emits its
serialized bytes alongside the parsed payload. Complete campaigns now compare
those bytes as well as results/refusals, dumps, phases and tapes. This closes
result ordering for the covered inputs; it does not close the separate
lossless stage-value boundary.

The new Python oracle freezes **57 complete refresh books**: eleven pairing
scenarios across current/stale and skip-current variants, empty books, ordinary
pairing-cache exceptions, second-rebuild trigger refusals, and legacy BLOB,
float and invalid marker values. Existing process and stage dumps are unchanged.
One additional trigger-backed book keeps freshness markers current after an
applied pair and independently asserts they are current at rebuild entry. It
pins the requirement to rebuild after applied pairs even in that state.
Native refresh tests compare every table, result byte, refusal, phase, clock
and UUID. Cancellation and pairing-panic tests prove rollback independently;
a file-backed WAL test observes both commits and post-reply restoration.

SQLite exception capture preserves ordinary failures separately from domain
application errors when constructing skipped-pairing summaries. A real
second-rebuild trigger failure retains Python's `IntegrityError` family and raw
SQLite message. Byte-valued freshness markers use Python's integer grammar and
retain exact invalid-byte representations. The accepted loader's mixed-type
revision refusal remains unchanged.

The broad native run passes **791 tests across 34 targets**. After the final
coordinator review, **54 journal library tests and four refresh tests** pass,
including exact application-error-code preservation and WAL restoration.
The focused Python run passes **342 tests** across refresh, process, renderer,
persistence, loader/BLOB and campaign harness modules. After adding the
current-marker witness, the four native refresh tests and all five Python
refresh cases pass again, including SQLite and SQLCipher. Journal and transfer
all-target clippy passes with warnings denied; formatting and all **239**
journal mutation anchors pass static checks.
The selected executable campaign kills **41 compiled mutations** with fifteen
passing baseline tests: twelve new refresh/order/refusal controls and 29
inherited process, integer and persistence controls. Its first run killed 40
and exposed the absent current-marker witness; after adding that book, the
focused baseline passes and the remaining mutation is killed. No inherited
fixture record changes. Both disposable mutation trees are removed. This is
selected evidence, not the complete inherited mutation catalog.
Production scope resolution and every journal route remain unregistered.

## Fresh complete-process campaigns after refresh composition

At `c14f2fa8e`, the revised unregistered probe passes **25,030/25,030** generated
comparisons: **24,000 books**, **1,030 permutations** and **1,207 exact refusals**.
All twelve generators complete. The separate retained-recording replay passes
**318/318** comparisons: **159 originals and 159 permutations**, with no
refusals. Both runs compare ordered result JSON bytes as well as complete
result/refusal values, every application-table scalar and rowid, REAL bits,
ordered phases and complete UUID/lifecycle-clock tapes. There are no failures
or exceptions. The accepted loader and refresh composition are included.

The native binary SHA-256 is
`0d881d04ab281c655ad62a01bff5bfac6328ce804ebcec0eed2aa6bc62ecedda`.
The [aggregate evidence](r4-journal-process-campaign-refresh.json) retains the
exact manifests, counts and per-suite totals without private recording names or
operands. Generated shards and logs are under
`target/r4-13/campaign-refresh-final/` and
`target/r4-13/logs/campaign-refresh-final.log`; recorded evidence is under
`target/r4-13/recorded-refresh-final/` and its matching log. All 101 generated
shards and both recording shards are complete and resumable against their
unchanged manifests. No verification process remains running after these runs.

These campaigns certify the stored-book adapter and fixed source/binary above.
They do not certify arbitrary malformed/lossless stage values, production scope,
registered envelopes/events, or the full inherited census/shadow/mutation gates.

## Four-book process CPU after merged refresh work

The [fresh raw record](r4-journal-process-refresh-measurements.json) contains five
alternating Python/native pairs on each of the four seeded benchmark books.
All **20/20** pairs match ordered payload JSON bytes, phase order and complete
scalar-exact database dumps. CSV and source-copy hashes were checked before the
run; source books were copied without changing the pairing worker's originals.
The binary hash is identical to the full campaigns above.

These runs use `--native-prebuild`, so real native repairs and cached pricing
execute. The benchmark books qualify for the no-op repair/pricing case; scope
is still injected through the private probe. CPU covers each complete child
from startup through response, deferred checkpoint, deallocation and exit,
including every native worker thread. Database copying and dump comparisons
are outside timing. This remains an unregistered process measurement; it does
not include the future registered CLI/daemon adapter or close its acceptance.

Each CPU cell is **best / median of five**, seconds. The ratio uses medians.

| Book | Python process CPU | Native process CPU | CPU speedup |
| --- | ---: | ---: | ---: |
| generic 2k | 1.627 / 1.658 | 1.164 / 1.178 | 1.41× |
| multi-wallet 2k | 2.061 / 2.107 | 1.658 / 1.734 | 1.22× |
| generic 20k | 12.154 / 12.293 | 11.317 / 11.411 | 1.08× |
| multi-wallet 20k | 21.628 / 22.047 | 15.492 / 15.822 | 1.39× |

The one-minute load ranges are 6.29–6.51, 6.67–6.89, 6.71–8.50 and 6.53–10.76,
respectively. Native wall medians are 0.894 / 1.282 / 8.258 / 11.017 seconds;
they remain above the four design budgets. The owner's 4× CPU target remains
unmet on all four books. The separate performance worker track remains open;
performance is not used as the reason to defer route registration here.

Final closeout regenerates the test classification inventory and passes the
required **57-test CI preflight**, formatting and all 239 journal mutation
anchor checks. The mutation trees and the main 2.5 GiB verification build tree
are removed after summarizing; the bound probe binaries, campaign shards and
logs remain under `target/`, about 2.1 GiB in total. All launched verification
jobs have finished.

## Remaining integration work

1. Bind the real `process_native` adapter to production scope resolution and
   `SystemSources` through the shared book service. Source repair/warning,
   legacy refresh/profile reload, cached pricing and lifecycle clocks are now
   composed; preserve the ordered source fields when emitting envelopes. Reuse
   setup's shared scope/selector resolver while preserving lossless stored
   profile values, rather than substituting the probe's injected profile.
2. Finish process/rendering acceptance gaps, including lossless stage values,
   exact refusals and the intended native boundaries. In particular,
   `process::stage_value` still refuses NaN, actual surrogate scalars and
   non-string keys, and transitional rendering/response decoding remains.
   R4-9 is fully accepted with no pending exceptions; these are process and
   R4-11 gaps, not loader exceptions.
3. The complete generated and retained-recording process campaigns pass above.
   Finish integrated frozen, census/shadow, codec/rollback and inherited mutation
   gates. The selected 41 mutation kills do not close the full inherited
   catalogs; future production route wiring needs its own named mutations.
4. Register plain CLI `journals process`, with exact ordered envelopes, native
   routing, zero fallbacks and route mutation tests; commit separately.
5. Register CLI `journals process --auto-pair` through the native R4-14 stage
   and atomic refresh coordinator, with the same envelope/routing/mutation
   requirements; commit separately.
6. Register desktop `ui.journals.process`, which accepts no arguments and uses
   `auto_pair=True`. Prove exact payload/event streams, cancellation, scope,
   zero fallbacks and route mutations; commit separately.
7. Register the scoped AI path with `auto_pair=False`. Preserve consent,
   book/input-version binding, cancellation and execution-thread boundaries,
   exact envelopes/events, zero fallbacks and route mutations; commit separately.
8. Keep performance acceptance on the separate worker track: measure the
   registered command from process start to exit against the owner's 4× target
   and four design budgets, then address R4-15/R4-16 authority work.

## Fourth continuation: stage representation integration

`t3code/r4-13-journals-process-4` starts at `649a72f1a`. It integrates the
renderer audit through `71307a23f`, native scalar ledger through `8225d58c1`,
and persistence through `8cb7947cc`, including their missing prerequisites.
Loader `aaf6216e3` is already represented by the inherited `718c49640`;
its duplicate cherry-pick is skipped. This remains an unregistered checkpoint.

The merged stage tips do not compile together without adapting persistence to
the renderer's borrowed entry/profile fields, borrowed-or-owned pricing facts,
and `JournalQuantity`/`JournalMsat` representations. The integration keeps
borrowed operands alive through storage, retains copy-on-write pricing edits in
the existing regression witnesses, and compares both small and wide msat
values without narrowing. Entry bindings preserve refusal precedence for
metadata, quantity and decimal formatting, including nonfinite quantities. Synthetic
pipeline responses now provide all mandatory empty response arrays required by
the stricter decoder.

The loader's existing native BLOB variant also needs explicit handling in the
renderer scalar primitives. Five immutable Python-derived byte witnesses pin
type, truth, escaped representation/text, raw JSON serialization refusal,
lossless oracle snapshots, and rejection by checked serde admission. These
primitive checks do not certify native numeric BLOB consumers or the complete
process handoff. Five named scalar mutations cover the new handling; four
existing render/persistence recipes are reanchored to the merged source. The
journal mutation wrapper now respects `CARGO_BUILD_JOBS`, defaulting to eight.

The combined all-feature journal verification passes **147 tests** across the
broad run and focused reruns, including the repaired seven-test pipeline suite,
three scalar tests, and SQLite/SQLCipher crash recovery. The native
ledger test compares all 990 inherited ledger states/refusals. All **66 Python
checks** pass for renderer/scalar regeneration and capture, rendering branches,
complete persistence replay, live native processing and ordered refresh. These
include the eight BLOB-consumer comparisons, active repairs/pricing, lifecycle
tapes, both refresh commits and abrupt writer recovery. Scoped all-target,
all-feature clippy passes with warnings denied; workspace formatting passes.
All **nine affected mutation recipes** are killed with **five passing
unmutated baselines**, zero survivors and zero build errors. All **875 entries**
apply across the ten journal catalogs; this bounded execution does not close
the full inherited mutation gate.
The required classifier update refreshes the disposition count table, and CI
preflight passes all **57 tests**. Local source-bound evidence is retained in
`target/r4-13/integration-4/`; disposable mutation and verification build trees
are removed after summarizing.

Registration prerequisites remain exactly as listed above. In particular,
`process::stage_value` still explicitly refuses NaN, actual surrogate strings
and non-string keys. The new native ledger reader has not replaced builder
assembly, diagnostic joins or persistence's serde operands. Production scope
resolution must retain stored profile values, and the shared book service must
bind the real lifecycle `SystemSources`; the probe's injected profile and tapes
do not satisfy that requirement. The earlier 25,030 generated and 318 recorded
comparisons belong to their recorded source and binary, and do not certify
this merge. Complete inherited campaigns and each route's authority, envelope,
event, zero-fallback and mutation gates remain open. No registered-command
performance result exists, and probe timings are not substituted for one.

## Fifth continuation: production binding

`t3code/r4-13-journals-process-5` starts at `502039979`. The shared book
adapter now binds `journals::service::process` to the existing setup selector
resolver and real transfer `SystemSources`. Resolution occurs under the writer
reservation and retains all stored profile fields through the loader adapter.
It does not substitute the probe's injected profile. The runtime reply handle
keeps the deferred checkpoint alive until the transport flushes its response.
Loader and process composition compile in the production feature set.
The binding passes 65 library checks and all four native process groups. The
complete-book/tape group replays each case through both injected and scoped
resolution. Foreign and ambiguous selectors refuse without writes; context,
ID and label selection preserve stored REAL values, including infinity.

This first boundary does not activate a route. The owner's registration strategy
allows whole-invocation Python fallback for native refusals before writes, or
after a proven byte-identical rollback. Successful admitted books must remain
native. Route activation and source-bound acceptance are recorded separately
below; the earlier campaigns and benchmarks remain historical evidence.

### Plain CLI registration boundary

Plain `journals process` now dispatches through the production book service.
`--auto-pair` remains explicitly unregistered in both ordinary dispatch and
the hybrid implementation probe. Successful results retain the ordered native
payload and publish/flush the terminal envelope before the deferred checkpoint.
No daemon kind, freshness job or AI tool is enabled by this boundary.

The hybrid launcher buffers native output until the invocation finishes. A
refusal may request whole-invocation Python replay only when the connection is
idle after rollback and hashes of the database, WAL and rollback journal equal
their values at service entry, including each file's absence. A second guard
resolves the selected book without bootstrap and compares those durable files
again after close, so bootstrap/migration commits and close-time changes cannot
escape the comparison. Missing/unresolvable initial books, changed selection,
unreadable or changed files and in-memory connections refuse replay. This
includes an opening migration that recreated a missing index and an opening or
closing checkpoint that changed the original durable files. SHM contains
lock-index state and is excluded. The product build emits its normal typed
error; the reserved replay signal exists only in the test build. Checkpoint/output errors
after a successful commit cannot request replay. The launcher retains bounded
passphrase bytes only in RAM and supplies fresh anonymous pipes to both children.

The new route tests bind clocks and durable UUIDs and compare complete typed
books, raw stdout/stderr and exit codes for generic/Austrian books on both
SQLite and SQLCipher. They include unchanged, changed, quarantined and empty
books, foreign/missing selectors, native served-by counts, reference replay,
faults after writes 1/8/15 and abrupt kills at those same writes. Reopened books
must pass integrity checking and match every starting table after a refusal or
kill. The route mutation catalog removes registration, dispatch, selector
forwarding and checkpoint completion; another removes the automatic-pairing
admission fence from ordinary dispatch and the hybrid probe. Each control names
its CLI boundary test.

### Refusal handoff for the lossless follow-up

The hybrid log counts `fallback_shape` as `<last phase>:<exception family>`;
it records no refusal message or book operand. The focused hybrid matrix and
descriptor cases have two successful native calls (one per codec) and nine
reference replays:

| Operand shape | Phase/family | Replays |
| --- | --- | ---: |
| Unsupported stored tax country | `preparing:ValueError` | 2 |
| Invalid stored gains algorithm, including three descriptor forms | `building:AppError` | 5 |
| Injected storage failure after write 8 | `storing:DatabaseError` | 2 |

The first two outcomes are the reference's exact refusals; the injected native
fault replays Python successfully. Valid generated/recorded books have not yet
been rerun through registration, so this bounded count does not certify their
zero-fallback gate. The known lossless follow-up includes NaN stage values,
actual surrogate scalars/keys, non-string stage keys, BLOB profile values the
setup resolver does not admit, and other malformed stored scalars/refusal
precedence. Those shapes must be measured separately; this table is not an
exhaustive census. Existing native cases that already preserve a BLOB consumer
are still admitted and must not be forced into fallback.

### Registration verification

The registered plain-route suite passes **44 cases**. Together with the shared
CLI-runner checks it passes **52 tests and six subtests**. Three inherited
hybrid checks also pass: usage routing, inherited descriptors to both runtimes,
and the default Python daemon. Their existing test fixture's `HOME` assignment
is filtered in a disposable runner; the account home is retained and XDG paths
and owner namespaces remain disposable. Faults and kills cover writes 1/8/15
on both codecs; the bootstrap-write cases refuse replay after index recreation.
All **10 native CLI tests** and **three production-service/guard tests** pass.
The five route mutations are all killed, with three passing unmutated baselines
and no build failures. Production compilation and scoped all-target,
all-feature clippy pass with warnings denied; workspace formatting passes.
The required classifier update completes and CI preflight passes **57 tests**.
Source/binary/log hashes and fallback counts are retained in
`target/r4-13/registration-5/verification-summary.json`; logs and the verified
harness binary remain there after disposable mutation/verification builds are
removed. No background verification job remains running at handoff.
The integrated generated/recorded campaigns have not been rerun through these
registered routes. These focused counts do not close R4-13 acceptance.

### Remaining continuation work

Register CLI automatic pairing, desktop processing (`auto_pair=True`) and the
internal scoped AI mutation (`auto_pair=False`) in separate verified commits.
The refresh coordinator's first rebuild currently commits before pairing, so
a late second-stage failure can retain that first rebuild; a byte-identical
guard must never replay such an invocation. Reconcile that boundary with the
owner's full-rollback requirement before enabling the flag. Desktop needs exact
progress/envelopes, argument/scope authority and cancellation tests. AI still
executes inside Python's consent-bound main-thread mutation dispatcher; adding
a public no-pairing daemon kind would not register that internal path.

Then rerun the 25,030 generated and 318 recorded comparisons, integrated
census/shadow and inherited mutation catalogs through the registered routes,
with source/binary manifests and zero fallbacks on valid books. Benchmark all
four registered commands/books from process start to exit against Python with
load recorded; the inherited probe measurements do not close that gate.

## Sixth continuation: invocation recovery contract

`t3code/r4-13-journals-process-6` starts at `08e99c524`. The full-invocation
rollback requirement allows Python's exact staged-commit semantics. The
reference `refresh_journals_step` commits the first stale rebuild in
`_journals_process_payload` before invoking pairing. A failed second rebuild
rolls back pending pairs and their candidate cache, while retaining the first
rebuild. A current starting book has no earlier commit to retain. Pairing
failures are caught, rolled back and reported as skipped; they do not abort
the refresh. This is existing Python behavior, also pinned by the inherited
`second-rebuild-failure-current0/1` fixtures. Python is unchanged.

The native coordinator already implements those commit boundaries. Its first
commit is therefore retained. The continuation adds a direct replay-guard
regression on both codecs: a failure after that commit cannot request a whole
invocation replay. It also adds live Python repros of both current/stale failure
cases on both drivers. The disposable native probe can pause at each progress
phase and each precise clock read, in addition to its existing per-write pause
and fault controls. Recovery checks cover every observed journal write, both
rebuilds, all four pairing clock calls and the boundaries before/after commits.
The sweep contains **208 abrupt kills**, **144 journal-write faults** and
**16 pairing-pass faults** across both codecs and current/stale starting books.
The checks reopen the book, check integrity and compare every typed application
row against the initial or exact Python first-commit snapshot as appropriate.
These controls belong only to the unpackaged example; route registration is a
separate boundary.

All **21 Python cases** pass across the full run and focused reruns, including
the exhaustive write/phase/clock kill sweeps, per-write faults and failures at
each pairing pass on both codecs. The four live reference failure cases retain
Python's exact first-commit tables. The oracle helper now preserves a supplied
driver's row factory, allowing those SQLCipher reference replays. All **five
Rust refresh tests** pass, including the new durable replay guard regression.
All-target, all-feature journal clippy passes with warnings denied, and
workspace formatting passes. The classifier update and required CI preflight
pass (**57 tests**). Registration and the remaining campaign gates remain open.

### CLI automatic-pairing registration

The CLI implementation probe and ordinary dispatch now admit `--auto-pair`.
The flag crosses the shared `Book::process_journals` contract and selects the
scoped native refresh service. Production binds transfer `SystemSources`;
test builds bind the same coordinator to clocks, UUIDs and write observers.
The service resolves the requested book once, fixes its workspace/profile IDs
and reloads the stored profile under each rebuild's writer reservation. It
preserves Python's first-commit policy and the deferred final checkpoint.
Plain processing keeps its existing result shape; automatic processing adds
the ordered `rebuilt` and `auto_pair` fields through the refresh coordinator.

The registered route matrix covers all eleven pairing scenarios, stale/current
projections, both codecs and an additional Austrian exact-pair case. Every
valid hybrid call must report Rust service with no fallback. Scope refusals
remain no-write operations. Late rebuild failure is injected with a temporary
trigger after schema admission, leaving the durable schema unchanged. A
separate registered write matrix discovers both commit boundaries from a
successful test-harness invocation before faulting or killing the first write
and the first/last writes of the second rebuild. Reopened books must match the
initial or exact live-Python first-commit snapshot, and a retained commit must
never authorize replay. The additional controls are compiled only into the
unpackaged test harness.

Verification passes **121 distinct registered CLI cases**: the full 111-case
run plus the expanded 16-case scope run, with six overlapping cases. All
**48 valid hybrid automatic-pairing calls** stay native, comparing raw output,
ordered envelopes and complete typed books without masking any result fields.
The inherited focused plain-route refusal checks retain nine replays: five
`building:AppError`, two `preparing:ValueError` and two injected
`storing:DatabaseError`. These focused counts are not an exhaustive census.
All **183 native CLI/journal tests** pass. All **seven route mutations** are
killed, with **four passing unmutated baselines**, no survivors and no build
failures. Production compilation, scoped all-target/all-feature clippy,
default-feature production clippy and workspace formatting pass.
The required classifier update completes and CI preflight passes all
**57 tests**. Source/binary/log hashes and bounded route/fallback counts are retained
in `target/r4-13/continuation-6/cli/verification-summary.json`. Verified binaries
and logs remain after disposable mutation and verification build trees are
removed. No background verification job remains running at handoff.

### Sixth-continuation handoff

CLI automatic pairing is the completed registration boundary. Desktop
`ui.journals.process` (`auto_pair=True`) still needs exact event/envelope,
scope/argument authority, cancellation and route mutation verification. The
internal scoped AI mutation (`auto_pair=False`) still needs registration inside
Python's consent-bound main-thread dispatcher; the chat loop remains Python.
A new public no-pairing daemon kind would not satisfy that internal path.

Then rerun **25,030 generated** and **318 recorded** comparisons through the
registered routes, including census/shadow and the inherited mutation catalogs
with source/binary manifests. Benchmark all four books from process start to
exit against Python with load recorded. Complete the lossless refusal census,
including NaN stage values, actual surrogate scalars/keys, non-string stage
keys, BLOB profile admission and malformed-scalar/refusal precedence. Earlier
probe campaigns and timings do not certify these remaining gates. R4-13
remains open.

## Seventh continuation: desktop registration

`t3code/r4-13-journals-process-7` starts at `fff1e85e0` and integrates the
nineteen R4-15 commits in `08e99c524..dfeaf1533`. The integration preserves
the sixth continuation's scoped CLI service and adds a separate resolved-profile
entry for freshness. Both enter the same native refresh coordinator. No
dependency changes are needed. The [freshness checkpoint](r4-journal-freshness-checkpoint.md)
continues to own wallet/queue consent and its development-only journal replay.

Desktop `ui.journals.process` registers with automatic pairing enabled and
`skip_rebuild_when_current=false`. It shares that resolved-profile service and
the foreground freshness execution slot. Like Python's direct desktop action,
it does not enqueue a freshness job or change source-state rows. It ignores no
processing arguments: nonempty objects and nonobjects receive the reference
validation envelope. The common `expected_scope` guard runs before dispatch.
Development selection requires an already open book and a nonempty UTF-8 string
request ID; unsupported identities retain whole-invocation Python selection.

The existing request worker opens and owns its SQLite/SQLCipher connection.
Its active book is captured on the request thread and checked on the connection
owner at journal phases, after pairing, during persistence and immediately before
commit. Cancellation before a first commit rolls back all pending writes;
cancellation or failure after the stale-first rebuild retains that commit and
rolls back pairs, candidate cache and the second rebuild. Lock, profile switch
and shutdown cancel and join the same worker before retiring its resources.
Competing foreground work returns retryable `project_operation_in_progress`.
The slot remains held until the terminal envelope is flushed and the deferred
checkpoint completes. A failure after publication cannot send a second terminal
response or authorize replay.

The desktop action constructs no freshness acquisition plan, network consent or
graph grant. Saved wallet/provider configuration does not authorize traffic.
Product binaries return native typed refusals and contain no Python bridge.
The desktop path does not enable the freshness job's development replay child;
the exhaustive lossless audit must decide any additional desktop handoffs.
Python remains the production authority until R4-16.

Unexpected reference exceptions retain Python-only traceback diagnostics. The
desktop oracle applies the established [shell comparison contract](rust-shell.md#import-run-checkpoint-r5)
only to `error.debug` traceback strings for `internal_error`; it pins every other
envelope field, all ordered progress and every typed table. A named comparison
scope test rejects changes to messages, retryability, details, codes or native
debug. The unsupported-tax-country case also reproduces and retains the Python
daemon's stderr traceback under the disposable test root. Python is unchanged.

### Desktop verification (eighth continuation)

`t3code/r4-13-journals-process-8` resumes the paused seventh worker at
`8b4266b56` and verifies its uncommitted desktop registration. Review made
three corrections. Desktop processing no longer inherits the freshness job's
900-second deadline, because Python's direct action has no time limit; only
cancellation, retirement or a context change stop it. The development selector
now admits every open book for `ui.journals.process`. Previously it rejected
an unresolved active profile, although the route already took Python's
plain-processing fallback for one. The desktop mutation runner now creates the
disposable tree's `target/` before using it as the pytest base directory.

A new four-shape test covers a missing profile, a profile in another
workspace, no stored context and a label-only selector. Each shape compares
ordered rows and every table with Python. One run passes **495 tests**: all
**142 desktop tests** on SQLite and SQLCipher plus the complete freshness
journal and product suites. All **nine desktop route mutations** are killed
with passing unmutated baselines. Classification and the 57-test CI preflight
pass. The eight inherited controls and the new
unresolved-context control each name one failing test. Formatting and
all-target, all-feature clippy pass with warnings denied for the touched crates.
Default-feature production clippy also passes. Static applicability passes for
all 44 mutation catalogs, including the re-anchored freshness lifecycle and
product entries.

### Assistant tool journal step

The assistant's `ui.journals.process` tool runs inside `ai.chat` through
`_execute_mutating_ai_tool` and `_run_scoped_ai_mutation` on the daemon's
main-thread connection. Its step is `_journals_refresh_payload(conn,
auto_pair=False)`, because the assistant's consent does not cover authoring
transfer pairs. An earlier note in this continuation kept the tool in Python
because R6 was out of scope. The owner has since brought R6 into scope.

`Book::refresh_journals` now takes the pairing choice explicitly. The desktop
route passes `true`; the assistant's step passes `false` through the same
resolved-profile service, freshness slot and cancellation checks. The native
daemon does not serve `ai.chat` yet, so the test-only `test.journals.refresh`
kind reaches the step on the daemon's own connection. It is not advertised
and not a substitute for the tool. `tests/rust_port/journal_ai_reference.py`
runs Python's exact step under the registered tests' clock and identity
bindings. `tests/test_rust_journal_ai.py` compares payload bytes or the typed
refusal and every table on SQLite and SQLCipher. It covers all eleven pairing
scenarios, current and stale, plus plain, refused and label-refused books.
A control shows that the oracle sees pairing when it is requested. All
**57 cases** pass. Consent, scope binding, result redaction and the chat
loop remain R6 work; the journal step itself is native and verified.

### Typed refusals stay native

The generated plain campaign below showed that every replayed refusal was a
typed `AppError`. Unguarded, 1,047 of 1,207 already matched Python's bytes and
books. The other 160 differed only in detail-key order: the RP2 label
refusal's `{"field","value","characters"}` reached the envelope through a
sorted JSON map. Native refusals now carry Python's insertion order to the
transport. Ledger-request refusals declare it, and pre-build refusals retain
their ordered application error. The CLI replay guard now requests development
replay only for exception families such as `ValueError` or `DatabaseError`;
typed `AppError` refusals are answered natively. Tests that pinned replay now
use an exception-family book, and a label-refusal book pins the detail order.
The freshness job's own replay policy is unchanged.

One run passes **438 tests** across the CLI, desktop, assistant-step and
freshness journal suites. Three new mutations and the re-anchored desktop
pairing control are all killed: the assistant step pairing, typed refusals
replaying, and sorted detail order. Path guards in the journal tests and
campaign scripts now resolve `target/`, which is a symlink to a separate disk.

### Registered-route CPU (eighth continuation)

The [route record](r4-journal-route-measurements.json) times the registered
routes from process start to exit with `scripts/benchmark-journal-routes.py`.
Each cell runs five alternating Python/native pairs on fresh copies of one
bench book. Python is the unchanged CLI or daemon under the registered tests'
clock binding. Native is a release build with the test harness, which binds
the same clock and identities and adds its write observer. CPU is the child's
user and system time across all threads, including startup, book open,
response, deferred checkpoint and exit. All **60 pairs** match exit status,
output bytes or ordered desktop rows, and every table scalar-exactly.

Each CPU cell is **best / median of five**, seconds; the ratio uses medians.

| Book | Route | Python CPU | Native CPU | Speedup | 1-min load |
| --- | --- | ---: | ---: | ---: | ---: |
| generic 2k | plain | 1.915 / 1.929 | 1.293 / 1.341 | 1.44× | 7.9–9.2 |
| generic 2k | `--auto-pair` | 2.097 / 2.156 | 1.290 / 1.316 | 1.64× | 8.5–9.2 |
| generic 2k | desktop | 2.135 / 2.233 | 1.516 / 1.623 | 1.38× | 8.9–11.8 |
| multi-wallet 2k | plain | 2.342 / 2.397 | 1.807 / 1.841 | 1.30× | 12.6–12.8 |
| multi-wallet 2k | `--auto-pair` | 2.753 / 2.811 | 1.855 / 1.909 | 1.47× | 11.5–12.8 |
| multi-wallet 2k | desktop | 2.671 / 2.721 | 2.065 / 2.092 | 1.30× | 9.6–11.7 |
| generic 20k | plain | 13.114 / 13.400 | 12.076 / 12.817 | 1.05× | 7.6–13.1 |
| generic 20k | `--auto-pair` | 16.319 / 17.023 | 12.333 / 12.933 | 1.32× | 12.4–20.8 |
| generic 20k | desktop | 16.297 / 16.739 | 13.516 / 13.693 | 1.22× | 11.8–23.9 |
| multi-wallet 20k | plain | 21.487 / 22.260 | 15.984 / 16.342 | 1.36× | 10.8–15.3 |
| multi-wallet 20k | `--auto-pair` | 26.306 / 26.587 | 16.130 / 16.213 | 1.64× | 7.5–11.7 |
| multi-wallet 20k | desktop | 26.298 / 27.823 | 17.561 / 17.964 | 1.55× | 6.2–109.6 |

Native wall medians are 1.10 / 1.42 / 9.48 / 11.35 seconds on the plain route.
The owner's 4× end-to-end CPU target is unmet on every cell. The last
multi-wallet desktop pairs ran while other workers raised the load sharply,
so that cell is the least reliable. This record does not attribute time to
phases; that remains with the separate performance track. Python retains
production authority until R4-16.

### Registered-route campaigns: plain route at `4afcc8d16`

`scripts/check-journal-route-campaign.py` materializes each generated or
recorded operand into a freshly created book, applies that book's pending
compatibility migrations once, and processes copies through the registered
route in unchanged Python (forked from a preloaded worker, with the registered
tests' clock binding) and in the test-harness binary. It compares exit status,
stdout and stderr bytes, `integrity_check` and every table scalar-exactly. A
guarded replay must leave the native book unchanged. The unguarded native run
is then also compared, to record whether the replay was necessary. Shards are
resumable against a manifest that binds the binary and sources. A deliberately
skewed native clock fails every compared operand on both CLI and desktop
routes. `--census` builds the perturbed operands defined in
`tests/rust_port/journal_route_census.py`.

The [evidence record](r4-journal-route-campaign.json) covers the plain route
with the binary built from `4afcc8d16` (before the native-refusal change
above). All **318 recorded** operands match natively. Of **25,030 generated**
operands, 23,544 succeed natively and 201 native refusals
(`custody_schema_migration_failed`) match Python. The 1,207 guarded replays are
typed refusals: 1,032 `book_network_review_required` and 15 pricing validations
already matched unguarded, and 160 RP2 label refusals differed only in
detail-key order. That order is now fixed.

Two causes remain outside the journal service. **64** books from
`gaps/blocked_competitor` keep unresolved legacy custody-migration issues.
Python retries that migration on every open and processes the book. Native
book open refuses it with `unsupported_schema_layout`
(`legacy_authored_custody`), as the store's compatibility gate requires.
**14** operands use the adapter's relaxed (constraint-free) tables, which
native schema admission refuses with `requires_python_migration` before any
write; the other 201 relaxed operands are migrated and match. Neither is a
journal parity defect, but both remain native refusals of books Python
processes.

### Registered-route campaigns: desktop and assistant routes

The second run uses the binary built at `f3cbf364c`, with every journal desktop
kind native, and the corrected assistant-step reference from `34e643869`. All
**318 recorded** operands match natively on the desktop, assistant, automatic
pairing and plain routes. On the **25,030 generated** operands, the desktop
`ui.journals.process` daemon route and the assistant's no-pairing step each
serve **24,952** natively. That is 23,544 successes and 1,408 typed refusals,
the latter now answered natively rather than replayed: 1,032
`book_network_review_required`, 201 `custody_schema_migration_failed` and 175
validations. Every row, envelope and table matches Python. The remaining 78
operands are the same store-admission refusals as on the plain route: 64
`gaps/blocked_competitor` books with unresolved legacy custody migration, and
14 relaxed adapter schemas. The [evidence record](r4-journal-route-campaign.json)
keeps the manifests and counts under the `v2-` runs.

The first assistant-route attempt found a harness defect, not a native one.
The reference opened the book outside its error handler, so Python crashed
where native returned the same typed refusal the chat tool reports. The
corrected reference reruns both assistant runs from scratch.

### Desktop journal reads and quarantine resolution

The owner's desktop test build needs every journal desktop kind native. The
daemon now registers `ui.journals.snapshot`, `ui.journals.events.list`,
`ui.journals.transfers.list`, `ui.journals.quarantine` and
`ui.journals.quarantine.resolve` alongside `ui.journals.process`.

`crates/journals/src/views.rs` ports the three journal reads from
`kassiber.core.ui_snapshot` with Python's queries and scalar conversions:
`int(x or 0)`, `float(x or 0)`, `float(msat_to_btc(...))`, argument coercion,
limit caps and the projection-freshness evaluation, including malformed
stored markers. `crates/journals/src/quarantine.rs` ports
`quarantine_review` and `quarantine_catalog`: reason categories and actions,
quantity blockers, waiting roots, groups, evidence, pair fingerprints, the
refresh-error report with operational redaction, and the presumption
summaries. Pair reviews come from a new flat `kassiber_transfers::pair_review_records`.
These reads open no new connection and never write.

`ui.journals.quarantine.resolve` keeps Python's argument order. It resolves
the book after validation, then the transaction by id or unique external id,
and requires a stored quarantine. One transaction applies the native audited
metadata update (exclusion or a manual price override), deletes the
quarantine row and invalidates journals when nothing changed. Derived
override prices use Kassiber's 32-digit decimal context, as Python's package
initialization sets. An optional plain native rebuild and the remaining
quarantine report follow.

Verification found and fixed a shared metadata defect. Grouped desktop saves
and history reverts wrote one `UPDATE` per column, so book triggers advanced
the chain-index revision clock once per column. Python writes one statement.
Both native paths now write one statement; the existing metadata and grouped
metadata parity suites pass again.

`tests/test_rust_journal_views.py` compares every request's ordered rows with
the unchanged Python daemon over plain, stale, quarantined, paired, swapped,
Austrian and context-less books, and over malformed freshness markers. It
also covers 54 admitted generated custody books whose quarantine pages
contain waiting roots, groups, actions and presumptions, and a short-received
pair with pair evidence plus a reported refresh error. Each session sends
every argument variant and refusal; reads must leave the book unchanged.
`tests/test_rust_journal_quarantine_resolve.py` compares refusals without
writes and each resolution, with and without rebuild, on both codecs. Only
the metadata layer's audit event ids and edit times are masked; these come
from the system clock in both runtimes. Python's reference stderr goes to a
file, because a full pipe once stalled a session with many tracebacks.

One run passes **136 tests** (one relaxed-schema generated book is skipped as
not admitted) across the two journal suites and the existing metadata and
grouped-metadata parity suites. All **11 route mutations** are killed: view
advertisement, pair type, the event limit cap, the input-version freshness
rule, waiting roots, pair fingerprints, reason categories, refresh-error
redaction, resolve advertisement, quarantine-row deletion and override-price
precision. Formatting, all-target/all-feature and production clippy, the
classifier update and the 57-test CI preflight pass.

### Seventh-continuation remaining work

Run all **25,030 generated** and **318 recorded** operands through the
registered routes, including census/shadow and every inherited mutation catalog
with source/binary manifests. Run the four-book benchmarks from process start
to exit against Python and record load. Complete the refusal census for NaN,
actual surrogate scalars and keys, non-string stage keys, resolver BLOB profiles,
malformed-scalar precedence and every other native refusal or development
fallback. Close supported shapes natively and retain each remaining cause.
The desktop boundary alone does not close any of those full-campaign gates.
