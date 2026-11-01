# Components performance checkpoint

Local branch `t3code/custody-perf-components`, based on `238422f90`. Multi-wallet
components fall from **1.5711 to 0.9302 s CPU** in the independent native comparison
(1.69×, 41% less CPU). The **10× custody target remains open**, as does full R4-3
typed-stage acceptance. Production changes stay inside `src/components/`; there
are no dependencies or changes to the separately owned evidence/projection stages.

## Implementation and compatibility

Native component support now retains typed observations, shared typed event keys,
interned transaction/wallet/asset strings, lazy canonical quantity hashes and
cached principal capacities per event, asset and direction. It avoids building
unused evidence payloads and detail hashes on a conservatively bounded common
path. The native support context contains no JSON values. Enrichment, authored
commitments, materialized legs and stage inputs/outputs retain their existing JSON
boundaries; this is not the complete typed components rewrite.

The shortcut requires signed-64 or NULL amounts and fees, valid quantity semantics
and nonempty normalized identifiers. Raw evidence must be NULL or an unescaped
string of at most 4,096 bytes with fewer than 64 opening container delimiters;
refund vout must be NULL or signed-64. These conservative bounds exclude Unicode
surrogates, excessive integer encodings and recursion errors in the unused detail
preimage. Other inputs run the original canonical observation admission, including
its historical error families. The quantity hash uses the same compact canonical
key order and UTF-8 string encoder. Hashes are memoized only within this invocation.

The authoring view still strips observation provenance before all-book enrichment
and fee normalization. Invalid observations retain their original skip/refusal
behavior. Duplicate transaction IDs retain the first insertion slot and the last
valid observation. The indexed event capacities preserve quantity-hash deduplication.
No cached observer authority becomes authoring evidence.

Grouping now uses indexed first-seen slots. Network connectivity indexes duplicate
IDs and allocation endpoints, borrows the unique legs, and compresses union-find
paths. It retains lexical root selection, duplicate replacement slots and issue
order. Hash-table iteration never determines output order. The support construction
uses the calling thread; existing independent enrichment maps retain their ordered
application. No new worker scheduling or cross-stage cache is introduced.

## Method and profile

Both committed-seed 20,000-transaction books run through the unregistered prepared
R4-13 process harness. Native pre-build R4-10 work and registered CLI/daemon routing
are outside this measurement. Generic has no authored components; multi-wallet has
300 active conversion components. The connectivity changes are exercised by the
corpora rather than by a large connected component in these benchmark books.

An initial instrumented baseline identifies native context construction as the
multi-wallet bottleneck: components 2.256 s, native context 1.901 s, enrichment
0.536 s, observation construction 1.056 s and materialization 0.225 s. These nested
spans overlap. Generic components cost about 0.00005 s. These initial heavy-host
samples are profiling evidence, not the baseline used for the paired gains.

Each comparison runs five alternating pairs on fresh copies of one migrated book.
Process CPU is user plus system across every process thread, measured with
`getrusage(RUSAGE_CHILDREN)`. A scratch-only preload maps native `Instant` spans to
`CLOCK_PROCESS_CPUTIME_ID`; Python uses its existing phase profiler with
`process_time`. All table spans come from each engine's fastest complete-process
trial, keeping stages together. Custody-output destruction after the custody span
remains in complete-process CPU. Python admission is included in components.

Toolchain: `mise exec rust@1.99.1 uv@latest`, locked Python 3.11, offline Cargo,
eight build jobs, release debug=1. Disposable data, caches and scratch stay under
this worktree's `target/`; account HOME is unchanged. No egress is enabled.

## CPU results

Seconds, Python/native, best complete process from five alternating pairs:

| Stage | Generic Python | Generic native | Multi-wallet Python | Multi-wallet native |
| --- | ---: | ---: | ---: | ---: |
| Admission | included below | 0.4763 | included below | 0.6362 |
| Components and migration | 0.2875 | 0.00003 | 2.5093 | 1.0095 |
| Evidence | 2.0409 | 1.3719 | 2.2351 | 1.6437 |
| Interpretation | 1.0901 | 0.2605 | 3.2714 | 0.7307 |
| Gaps | 1.3076 | 0.1659 | 1.3595 | 0.1692 |
| Quantity | 0.8788 | 0.6631 | 4.4020 | 0.8373 |
| Finalized projection | 0.4935 | 1.2102 | 0.5741 | 1.4904 |
| **Custody total** | **6.0984** | **4.2127** | **14.3513** | **6.6315** |
| **Complete process** | **14.6272** | **11.8191** | **24.8352** | **16.4531** |

Custody speedups are **1.45× generic / 2.16× multi-wallet**. One-minute loads are
28.11–41.91 generic and 21.96–32.56 multi-wallet. These shared-host measurements
do not establish idle-host acceptance.

Independent saved-native-baseline/candidate comparison:

| Stage | Generic before | Generic after | Multi-wallet before | Multi-wallet after |
| --- | ---: | ---: | ---: | ---: |
| Admission | 0.4799 | 0.4741 | 0.5639 | 0.6209 |
| Components and migration | 0.00003 | 0.00003 | 1.5711 | 0.9302 |
| Evidence | 1.2715 | 1.2633 | 1.6239 | 1.4005 |
| Interpretation | 0.2298 | 0.2322 | 0.7583 | 0.7082 |
| Gaps | 0.1625 | 0.1604 | 0.1881 | 0.1863 |
| Quantity | 0.6914 | 0.6676 | 0.8925 | 0.9279 |
| Finalized projection | 1.0967 | 1.0826 | 1.3513 | 1.2802 |
| **Custody total** | **3.9946** | **3.9460** | **7.0575** | **6.1728** |
| **Complete process** | **10.9957** | **11.5374** | **15.9174** | **15.5865** |

Multi-wallet observation construction falls from 0.7574 to 0.2691 s; enrichment
is 0.3628/0.3903 s and native context 1.3296/0.7725 s. Custody gains are
1.01× generic / 1.14× multi-wallet. Loads are 24.04–33.46 and 34.23–59.98.

## Whole-process regression investigation

The preceding performance-5 multi-wallet median increased by 0.192 s, although
four of its five paired complete-process trials improved. The one positive
paired delta, about 1.373 s, arose mostly outside custody. This does not isolate
a repeatable components-caused process regression.

This batch's unpinned process medians are 12.6957 → 13.3680 s generic and
16.2520 → 16.2225 s multi-wallet. Generic's component path is empty. To check
placement sensitivity, a separate five-pair comparison per book pins both engines
to CPU 12, retaining every trial. Those process medians improve to
16.4438 → 15.8452 s generic and 18.8538 → 17.8184 s multi-wallet. Loads are
28.84–36.30 and 27.34–38.93.

The pinned best-complete-process multi-wallet components improve
1.4457 → 0.9775 s and process CPU improves 17.6306 → 16.1694 s, while custody
CPU increases 7.2283 → 7.7326 s because other stages vary. Generic best process
is 15.0425 → 14.9479 s and custody is 4.9971 → 5.3301 s. These controls support
caution about unpaired median regressions; they do not prove full-custody
non-regression under controlled idle-host conditions. No unrelated pipeline code
was changed to mask the variation.

All thirty paired comparisons agree on complete scalar-exact database dumps,
payloads and phase sequences. Every trial, load observation, aggregated nested
span, source/binary hash and local raw-log hash is retained in
[`performance-components.json`](performance-components.json).

## Verification

All **24,000 generated books and 1,030 permutations** pass: 50,060 byte comparisons
and 178 matching refusals. All twelve generator gap signatures and the eleven
interpretation fixtures match. Per-scenario activity and distinct active-input
counts are retained in the machine-readable record.

All **159 recordings and 159 permutations** pass (636 byte comparisons). All
**626 Rust custody tests**, including frozen vectors and the two new typed-support
regressions, pass. Formatting and clippy with warnings denied pass. The corpus
run precedes a test-only zero-quantity case addition; final rebuild, binding sync,
complete Rust tests, formatting and clippy pass afterward. Production source is
unchanged between those runs.

The catalog retains all 2,334 inherited recipes, re-anchors two typed-native
expressions, and adds 18 typed-support recipes: **2,352 total**, of which **89**
target touched files. The initial touched run killed 88/89; removing the raw-size
bound alone survived because the parser falls back to opaque raw text. That new
recipe is replaced with an effective canonical detail-admission mutation. The
initial result remains separately recorded and is not counted as a kill or
relabelled with the new catalog fingerprint.

The full catalog contains 2,331 Rust and 21 Python harness recipes. Its first
Python recipe exposed an audit-wrapper error: the wrapper recorded only Cargo
outcomes. It stopped before recording that recipe, preserving 2,025 valid kills.
The corrected wrapper compiles the mutated Python files and requires positive
pytest failure counts. Source and catalog fingerprints remain unchanged; shard 6
resumed from its 243 recorded kills. Interrupted logs and the pre-fix checkpoint
are preserved separately. This was an audit failure, not a production-code failure
or a mutation survivor.

The T3 server later restarted after disk metadata exhaustion during shard 7. The
persisted checkpoint contained 2,307 verified kills. The first verified local
source commit, `6f7b3c439`, preserves that boundary. After recovery, all remaining
45 recipes pass against the same source and catalog fingerprints. Interrupted
logs and checkpoints remain available; no interrupted or unrecorded outcome is
counted as a kill.

The final candidate kills **2,352/2,352 catalog recipes**, including all **89
touched-file recipes**, across the touched batch and eight resumable catalog
shards. Rust mutants pass an offline `cargo check --locked --lib --tests` before
their assigned tests fail. Python mutants pass compilation before positive pytest
failure counts are recorded. Every assigned test has a positive failing-test
count. There are no survivors or build/test errors in the completed runs. Resumed
batch counts exclude recipes already counted in the touched batch.

Classification update passes without a disposition change, and CI preflight
passes all **57 tests**. Final source and catalog fingerprints match the completed
mutation state. Disposable verification builds, mutation source/build trees and
scratch copies are removed after summarizing their results. Saved benchmark
binaries/books, raw CPU pairs, corpus activity and small logs/state remain under
`target/custody-perf-components/`. No task or background test job remains running.

Resumed verification uses **six build jobs and the shared uv cache**, with no
worktree `UV_CACHE_DIR`. The earlier benchmarks and gates used eight build jobs.

## Remaining work

The Python/native campaign leaves custody at **4.2127 s generic** against 0.51 s
and **6.6315 s multi-wallet** against 1.21 s: 8.26× and 5.48× the respective
budgets. Even the improved components alone consume most of the multi-wallet
custody budget. Evidence, quantity and finalized projection remain substantial
costs and are separately owned.

Complete R4-3 requires typed materialized component inputs/outputs and enrichment,
with no JSON inside the stage, followed by integrated whole-pipeline verification
and measurement. This checkpoint retains those boundaries and does not close
R4-3, the global 10× target or idle-host process non-regression acceptance.
