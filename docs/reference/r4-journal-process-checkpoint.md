# R4-13 journal process checkpoint

This is a **test-only, unregistered checkpoint** on
`t3code/r4-13-journals-process`, based on `b5eb38ea8` with loader rounds 3–8
(`0b78605a0..c5966c1ff`) integrated. It does not complete R4-13 or switch
production authority. CLI `journals process`, `--auto-pair`, daemon
`ui.journals.process`, desktop refresh and AI mutations retain their existing
routing.

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
4. Register plain CLI `journals process`, verify its exact envelope, routing
   and zero fallback, and commit that boundary. Keep `--auto-pair` on Python
   until the R4-14 contract is integrated.
5. Register daemon processing only after reconciling the pairing contract and
   proving exact payload/event-stream parity, cancellation/scope and fallback
   behavior; commit separately. The current desktop kind accepts no arguments
   and calls `_journals_refresh_payload` with its default `auto_pair=True`.
   The scoped AI path explicitly passes `False`. A non-pairing replacement
   cannot silently stand in for both. R4-14 and the existing consent boundary
   must be preserved.
6. Measure the registered command from process start to exit and meet the
   owner's 4× end-to-end target and the four design budgets; then address
   R4-15/R4-16 authority work.
