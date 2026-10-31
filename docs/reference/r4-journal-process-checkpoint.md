# R4-13 journal process checkpoint

This is a **test-only, unregistered checkpoint**. The original
`t3code/r4-13-journals-process` starts at `b5eb38ea8` with loader rounds 3–8
(`0b78605a0..c5966c1ff`) integrated; the continuation
`t3code/r4-13-journals-process-2` starts at `925800ebe`. It does not complete R4-13 or switch
production authority. CLI `journals process`, `--auto-pair`, daemon
`ui.journals.process`, desktop refresh and AI mutations retain their existing
routing.

## Continuation stage snapshot

The local continuation `t3code/r4-13-journals-process-2` starts at
`925800ebe`. Its initial handoff snapshot is:

| Stage branch | Committed tip inspected | Integration |
| --- | --- | --- |
| `t3code/r4-9-loader-9` | `c5966c1ff5bbb1bbad1a4b073adf2d10f468dc81` | Already inherited through loader-8; `loader-8..loader-9` is empty. |
| `t3code/r4-10-prebuild` and `t3code/r4-10-prebuild-2` | `bc9e126d3739b15945ba67542cdc5e30bd09abab` | Cherry-picked cached pricing; no later committed handoff was available. |
| `t3code/r4-11-render-3` | `197f6cdecc2a8ed12f1ee78a32ef92264bc15c25` | Cherry-picked boundary hardening, preserving newer loader representation and process mutation recipes. |
| `t3code/r4-12-persist-2` | `b5eb38ea8cb764675322e61e9b957a57deb947c6` | Already an ancestor; no new committed persistence changes. |

The pricing cherry-pick retains the continuation's newer shared-column loader,
admission tests, process example and checkpoint documentation. Its older
loader build fixes do not replace loader rounds 3–8. No new dependencies or
lockfile changes are needed. Cached pricing alone does not implement either
source-overlap repair or drifted legacy-pair refresh; neither may be silently
omitted from a registered command.

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

The test-only process adapter now optionally runs real native cached pricing.
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

Production registration still requires the following concrete work:

- R4-10 must supply scope/profile resolution and both real repairs:
  `handlers._repair_journal_source_overlaps` (including warning construction,
  metadata history, address-list trimming and invalidation) and
  `custody_authored_migration.refresh_drifted_legacy_pair_components` (including
  refreshed profile state). Neither implementation is present in the committed
  pricing handoff. A no-op implementation would change accounting.
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
The example requires `prepared_noop: true`, an injected profile, clock and UUID
tape, and restricts physical database paths to this worktree's `target/`.
Live Python comparisons prove that the real pre-build stages leave their
fixture books unchanged before using that seam. Synthetic benchmark books
likewise must produce zero pre-build writes and zero pricing/repair counts.
This establishes prepared-book parity only; it does not establish R4-10.

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
render/persistence boundaries. It still converts native rows for typed
admission, materializes JSON stage operands, and calls the text ledger
boundary. It does **not** satisfy the design's no-JSON production-path gate.
Floating NaN, surrogate strings and non-string stage keys at the renderer seam refuse
explicitly pending lossless R4-11 integration. Other loader legacy-value and
refusal limitations remain those in the loader checkpoint.

## Verification scope

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

The 24,000 generated books with per-scenario activity, 1,030 permutations,
159 retained requests and their permutations/fresh recordings, census and
shadow campaigns have **not** been rerun for this integration. Sampled and
inherited component evidence cannot substitute for those gates. No served
shape or zero-hybrid-fallback result is claimed.

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

## Remaining integration work

1. Integrate and verify real R4-10 resolution, repairs and cached pricing.
2. Integrate the completed R4-9/R4-11/R4-12 acceptance handoffs, including
   lossless stage values, exact refusals and the intended native boundaries.
3. Run all full-pipeline generated, replay, frozen, census/shadow, codec and
   rollback gates and the entire inherited mutation catalog. Complete wiring
   branch/mutation coverage and close performance gaps.
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
