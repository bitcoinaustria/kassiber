# R4-9 native custody loader checkpoint

This is an **unaccepted, test-only checkpoint**, updated 2026-10-31. Loader-3
continues `0b78605a0` (loader-2). Generic 20k now meets the 0.30 s target at
**0.289 s process CPU / 0.291 s wall**, including admission. Multi-wallet 20k
measures **0.429 / 0.433 s**. Python retains every production load and journal
route. Exhaustive loader mutations, lossless legacy values/refusals and the
remaining adversarial acceptance work still keep R4-9 open.

## Representation and authority

`crates/journals/src/loader.rs` exposes `load(connection, profile, as_of)` only
under tests or `test-harness`. The connection belongs to an already opened
`kassiber_store::Book`. Loading requires the caller's SQLite write transaction,
reads pending writes, and never begins, commits, rolls back, repairs or updates
the book. Existing canary tests preserve caller rollback ownership on success
and refusal. No product, daemon or AI route registers the loader probe.

SQL rows populate custody records directly. A record now holds a shared,
sorted, immutable `Columns` layout plus a vector of values. Query preparation
allocates field names once; rows clone only the `Arc` handle. Name lookup,
iteration, missing-field behavior, duplicate JSON keys and serde object output
preserve the former `serde_json::Map` contract. Explicit `into_map()` calls
materialize compatibility objects where existing stages still need JSON.
The JSON adapter and the native loader use the same record type. Rust report
and transfer snapshot builders also adapt to this shared contract.

The integrated typed-row checkpoint retains its direct `BookSnapshot` admission
and the independent complete-v1 byte gate. See the [R8 integration audit](rust-backend-r8-integration.md#combined-performance)
for the prior combined measurements; the following source-line timings precede
this integration and do not establish new combined acceptance.

The generic-20k transactions, rate and gap projections previously owned
1.70 million field keys across 60,000 maps. Those projections now share three
column layouts. Network observations still materialize JSON objects for the
existing network consumer. This is a transitional row representation, not the
plan's eventual lossless `SqlValue`/`BookSnapshot` boundary. INTEGER and REAL
remain distinct, and BLOB, invalid UTF-8 TEXT and nonfinite REAL still have
unfinished legacy/refusal fidelity.

Admission memoizes only successfully checked, identical configuration strings
within one validation call. Every different string and every object still goes
through the same configuration allowlist. Wallets, transactions, anchor wallets,
channels, gap rows and anchor objects retain both configuration checks, and
transaction principal/fee admission still requires exact signed-64 msat.
No memo survives a snapshot validation or grants accounting authority.

Ownership, descriptor derivation, receipt sources, the bounded >50k worklist,
error precedence and SQL selection/order remain the loader-2 implementation.
Column-order metadata and ordered derived facts remain solely for the v1 byte
gate. The loader adds no egress, dependency, durable storage or production
runtime requirement. `Cargo.lock` is unchanged.

## Paired timing and profile

[`r4-native-loader-3-bench.json`](r4-native-loader-3-bench.json) contains all
20 pairs, generator manifests, byte lengths/hashes, five repetitions of the
four-query SQL floor, the release binary hash and sampling summaries.
The disposable books are copies of loader-2's committed-seed generator books:
generic seed 1 and multi-wallet seed 7, at 2k/20k. All four first pass the
complete Python/native byte gate; SQLCipher parity is checked separately.

`scripts/benchmark-custody-loader.py` alternates Python/native ordering for
five back-to-back pairs per book. CPU is process user+sys. Each implementation
selects its best complete CPU sample; wall time comes from that same sample.
Native total includes admission on the directly built snapshot. Book open,
BEGIN and the test-only serializer are excluded. No snapshot conversion is
moved outside the timer: consumers receive the shared records directly.

| Book | Python CPU / wall | Native load CPU / wall | Admission CPU / wall | Native total CPU / wall | 1-minute load |
| --- | ---: | ---: | ---: | ---: | ---: |
| generic 2,000 | 0.113 / 0.114 | 0.026 / 0.026 | 0.001 / 0.001 | **0.027 / 0.027** | 11.75–11.75 |
| multi-wallet 2,000 | 0.150 / 0.151 | 0.051 / 0.052 | 0.002 / 0.002 | **0.053 / 0.054** | 11.53–11.53 |
| generic 20,000 | 1.168 / 1.177 | 0.280 / 0.282 | 0.009 / 0.009 | **0.289 / 0.291** | 11.22–11.41 |
| multi-wallet 20,000 | 1.443 / 1.454 | 0.406 / 0.409 | 0.023 / 0.024 | **0.429 / 0.433** | 11.10–11.17 |

Compared with the inherited loader-2 measurements, generic20k CPU falls from
0.505 to 0.289 s (43%) and multi-wallet20k from 0.687 to 0.429 s (37%). These
are separate shared-host runs, not controlled before/after pairs. The current
Python/native comparisons above are paired. The four-query SQL floor is
0.073 s CPU / 0.073 s wall at generic20k and 0.090 / 0.090 s at multi-wallet20k;
it excludes ancillary queries and decoding.

Profiling preceded edits. The copied baseline probe's SHA-256 equals
loader-2's recorded `47791880351152ce1a70b9c64863c2ad1cc6e7b1dd3c07439b0eb63f2cbaca4a`.
`pmp.py` collected 400 baseline samples and 362 optimized samples on repeated
timing-only generic20k requests. The optimized loop finished before its requested
400th sample. Both profiles include book open and final drop, and percentages
overlap. Optimized symbols lack source-line detail; unresolved libc frames
account for 35.8% / 29.3% of self samples, so these are not allocator-only costs.

| Inclusive frame | Baseline | Shared columns |
| --- | ---: | ---: |
| Row construction | 58.5% | 69.9% |
| SQLite stepping | 20.2% | 25.1% |
| Admission | 9.8% | 2.5% |
| Scalar decoding | 10.5% | 25.1% |
| Python-compatible float formatting | 3.5% | 9.1% |

B-tree bulk construction's self samples fall from 8.8% to 0.6%. Row decoding,
SQLite stepping, float formatting and remaining JSON consumers now dominate a
smaller total. The measured SQL floor is not a proof of the loader's minimum.
Raw profiles and disposable books stay under `target/r4-loader-3`.

## Verification

The shared-record change passes the 612-test custody/journals suite, followed
by all three focused shared-record/admission tests. The live Python database
oracle passes **44 tests**, including SQLCipher root/pending-write copies and
both >50k worklist paths. All four benchmark books retain exact bytes:
6,793,240 / 9,238,672 / 68,023,010 / 76,513,056 bytes respectively.
Workspace/all-target offline clippy with the loader's `test-harness` and
`-D warnings` passes. The full Rust workspace suite passes **1,504 tests**,
with no failures or ignored tests. Regenerating the **37 frozen SQL fixtures
and four frozen refusals** against Python produces no changes. The twelve
inherited raw generator suites pass all
**24,000 books and 1,030 permutations**. Read-only retained replay compares
**159 requests and 159 permutations**, with zero differences and zero
pre-request refusals.

The loader mutation catalogue grows from 59 to 69 entries. Two existing
configuration-marker recipes are mechanically re-anchored to vector assignment.
The ten new recipes target shared columns, lookup/value pairing, serialization
and configuration admission. Its runner includes custody because the shared
record and admission witnesses now live there. All **69 loader mutations**
build and are killed by their named witnesses; all ten unmutated witness filters
pass first, with no survivors or runner errors. This is still a partial branch
catalogue. The inherited custody catalogue contains 2,292 entries in this
checkout; its profile projection recipe is mechanically re-anchored to the
explicit compatibility conversion. Applicability passes for both catalogues.
The re-anchored `projection8-profile-field` mutation also builds and is killed
by its named witness after an unmutated pass.
The complete custody mutation campaign has not been rerun at this checkpoint.

The required Rust-port inventory update completes without an inventory diff,
and the CI preflight passes **57 tests**. Rust formatting and `git diff --check`
pass. A manual second review covers the shared record, admission order, direct
reader and compatibility conversions.

The loader-2 census/shadow evidence remains inherited: eight shard runs with
1,659 reported exact matches, 37 frozen SQLite fixtures and four frozen exact
refusals. See the historical [loader-2 timing records](r4-native-loader-2-bench.json)
and `0b78605a0` for its full checkpoint. New results do not imply that those
entire shard pairs were rerun.

Verification uses Rust 1.99.1, locked Python 3.11, offline Cargo and
`MATURIN_NO_INSTALL_RUST=1`, without changing `HOME`. Only one test job runs at
a time; scratch and temporary data stay under this worktree's `target`.
After writing verification summaries, the debug/release build trees and mutation
scratch trees are removed. The standalone probe, synthetic books, profiles and
logs remain under `target/r4-loader-3`.
No push, PR, external comment or production authority switch is in scope.

## Remaining acceptance work

1. Finish named mutation kills for every loader branch; 69 entries are still a
   partial catalogue. Repeat the full inherited custody mutation campaign after
   all remaining behavior changes; this checkout has 2,292 entries, rather than
   the 2,285 in the task brief.
2. Finish lossless legacy `SqlValue` storage and exact refusal order/family/
   message, including BLOB, nonfinite REAL, invalid UTF-8 and unusual SQLite
   storage/config/descriptor scalars across every read column. Existing early
   adapter refusals do not establish Python fidelity. A focused repro also
   confirms the inherited safe-row omission: adding a physical TEXT column
   named `_closed_confirmation_observation` to `transactions` makes native
   retain that column, while Python removes it. This is not introduced by
   shared columns and remains an acceptance blocker.
3. Expand adversarial descriptor and source-index coverage beyond the inherited
   high-water, aliases, retired-policy, cache-isolation and escaped-identity
   cases. Complete remaining parity/review gates before accepting R4-9.

## R4-13 integration contract

R4-13 must open through `crates/store`, hold `BEGIN IMMEDIATE` and its build
savepoint, run R4-10 repairs/pricing, then call the loader on that same connection
with the exact ordered profile and injected `as_of`. It must consume the eventual
lossless typed snapshot directly, roll back refusals/panics without refreshing
freshness markers, and commit only after successful persistence. CLI/daemon/AI
routes must not call this test-only prototype or adopt its serializer as a
production boundary. Accounting admission and watch-only/no-egress rules remain.
