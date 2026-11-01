# Custody sharing and mutation continuation checkpoint

Local branch `t3code/custody-perf-5`, based on `19792ea1a`. The **10× Python target
remains open**. All tracked changes are inside `crates/custody`; fusion and adapter
boundaries remain separately owned.

Canonical event slots and selected legs now borrow their observations. Internal graph
scope and event identity readers borrow decoded values and share immutable embedded JSON
text parses through standard-library `Rc`. Owned readers still return independent
values. Generic object conversion and transfer identity conversion retain separate
caches, including their different nonfinite behavior. Minimum tie selection, refusal
ordering and output bytes are unchanged. There are no new dependencies.

## Method

The two committed-seed 20,000-transaction books use the same unregistered, prepared
R4-13 process harness as the preceding checkpoint. Native R4-10 pre-build work and
registered CLI/daemon routing remain outside the measurement. Rust is 1.99.1, Python is
locked 3.11, Cargo is offline with eight build jobs, and native binaries use release
builds with debug=1.

Each comparison contains five alternating pairs on fresh copies of one migrated book.
Process CPU is user plus system time across all threads, measured with
`getrusage(RUSAGE_CHILDREN)`. A scratch-only preload maps native `Instant` spans to
`CLOCK_PROCESS_CPUTIME_ID`; Python uses the existing phase profiler with `process_time`.
The tables retain every stage from the fastest complete-process trial for each engine,
rather than mixing stage minima. Nested spans overlap. Output destruction after custody
returns remains in complete-process CPU.

All twenty pair comparisons agree on complete scalar-exact database dumps, payloads and
phase sequences. Raw trials, CPU/wall times, load observations, nested stage spans and
source/binary hashes are retained in [`performance-5.json`](performance-5.json). These
shared-host measurements do not establish idle-host acceptance.

## CPU results

Seconds, Python/native:

| Stage | Generic Python | Generic native | Multi-wallet Python | Multi-wallet native |
| --- | ---: | ---: | ---: | ---: |
| Admission | included below | 0.4916 | included below | 0.5691 |
| Components and migration | 0.2609 | 0.00003 | 2.3287 | 1.3651 |
| Evidence | 1.9160 | 1.2481 | 2.1050 | 1.3245 |
| Interpretation | 1.0249 | 0.2132 | 3.0304 | 0.6840 |
| Gaps | 1.2049 | 0.1663 | 1.2268 | 0.1738 |
| Quantity | 0.8032 | 0.6567 | 4.2863 | 0.7975 |
| Finalized projection | 0.4681 | 1.0552 | 0.4829 | 1.1251 |
| **Custody total** | **5.6780** | **3.9001** | **13.4600** | **6.1272** |
| **Complete process** | **13.6440** | **10.5425** | **22.1908** | **13.7278** |

Custody speedups are 1.46× generic and 2.20× multi-wallet. Complete-process speedups are
1.29× and 1.62×. Python admission is included in components.

Python/native, generic-20000: median custody CPU 5.7205/3.9455 s; one-minute load
14.48–23.36.

Python/native, multi-wallet-20000: median custody CPU 14.1958/6.1546 s; one-minute load
14.74–21.02.

before/native, generic-20000: median custody CPU 4.3200/4.2872 s; one-minute load
18.98–26.16.

before/native, multi-wallet-20000: median custody CPU 6.7331/6.2783 s; one-minute load
14.61–19.36.

Independent native-baseline/candidate comparison:

| Stage | Generic before | Generic native | Multi-wallet before | Multi-wallet native |
| --- | ---: | ---: | ---: | ---: |
| Admission | 0.4756 | 0.4722 | 0.6062 | 0.5222 |
| Components and migration | 0.00003 | 0.00003 | 1.3920 | 1.3402 |
| Evidence | 1.3901 | 1.2271 | 1.4694 | 1.2927 |
| Interpretation | 0.2157 | 0.2277 | 0.7362 | 0.7253 |
| Gaps | 0.1842 | 0.1582 | 0.2085 | 0.1761 |
| Quantity | 0.6306 | 0.6592 | 0.8072 | 0.7932 |
| Finalized projection | 1.0914 | 1.0893 | 1.1513 | 1.1356 |
| **Custody total** | **4.0435** | **3.9003** | **6.4621** | **6.0773** |
| **Complete process** | **11.0677** | **10.9218** | **14.3431** | **13.8007** |

Custody gains are 1.037×/1.063×. Evidence improves from 1.3901 to 1.2271 s generic and
1.4694 to 1.2927 s multi-wallet. Whole-process medians are 12.0412/11.9241 s generic and
14.8689/15.0607 s multi-wallet; the latter is slower despite the best-trial custody
gain. The complete alternating samples delimit these modest gains.

## Verification and mutation continuation

The seven remaining inherited shards selected 292, 291, 292, 291, 291, 279 and 92
recipes. The first six killed every selected recipe; the final shard killed 90 and
exposed `R7-price-exponent` and `R7-price-floor`. Their assigned base-row vector had
moved to the field-specific price validator and no longer exercised the targeted
primitive guard. Direct positive/negative boundary regressions now kill both: exponent
308/309, the exact `1e-13` floor and values immediately below it, plus canonical zero
handling. Production price behavior is unchanged. All 2,333 inherited recipes are
accounted for across the original source and the separately fingerprinted test
correction; old kills were not relabelled with a new fingerprint.

The final catalog retains all inherited IDs, anchors, replacements and rules, changes
only those two test assignments, and adds a cache-kind isolation recipe, for 2,334
total. The new regression also proves owned-output mutation isolation for both readers.
All 624 Rust custody tests, including frozen vectors, pass. Formatting and clippy with
warnings denied pass.

All 24,000 generated books and 1,030 permutations pass, with 50,060 byte comparisons and
178 matching refusals. All twelve gap-signature fixtures and eleven
interpretation-signature fixtures agree. Per-scenario activity and distinct active-input
counts are retained in the machine-readable record.

All 159 recordings from `/home/tg/.cache/kassiber-verify/custody-recordings/` and their
159 permutations pass (636 byte comparisons).

The final candidate kills all **2,334/2,334 catalog recipes**, including all 87
touched-file recipes, with no survivors or build/runner errors. The touched and
remaining-catalog baselines pass 42 and 395 named test filters respectively. Each mutant
compiles with offline `cargo check --locked --lib --tests` before its assigned tests
record positive failing-test counts; compilation failures are not kills.

The required `classify_tests update` passes without an inventory disposition change.

The required CI preflight passes all **57 tests**. All eight serial verification steps
exit zero. Local log paths and their SHA-256 hashes are retained in the machine-readable
record.

The final source and catalog fingerprints match the completed mutation state. The
catalog retains every inherited rule, anchor and replacement; only the two price-test
assignments change, and the cache-isolation recipe is added.

Disposable mutation and verification build/source trees are removed after summarizing
their results. Saved benchmark binaries, raw paired trials and small logs/state remain
under `target/custody-perf-4/` and `target/custody-perf-5/`. No test or background job
remains running.

## Remaining budget blockers

The Python/native paired trials leave generic custody at 3.9001 s against 0.51 s (7.65×
the budget), and multi-wallet at 6.1272 s against 1.21 s (5.06×). This is not R4-3/R4-7
acceptance.

Generic evidence alone costs 1.2481 s and finalized projection 1.0552 s: each exceeds
the entire generic budget. Quantity adds 0.6567 s and admission 0.4916 s. Multi-wallet
components alone cost 1.3651 s, already above its entire budget; evidence adds 1.3245 s,
projection 1.1251 s, quantity 0.7975 s and interpretation 0.6840 s. The unchanged owned
stage-output representations and their repeated materialization remain the main targets.
The data do not establish that 10× is impossible; they show why this kernel batch does
not reach it.

The coherent native subspans narrow the next batch: generic/multi-wallet canonical event
construction costs 0.6548/0.6633 s; enriched rows cost 0.3031/0.2718 s. Multi-wallet
`components.native` costs 1.1666 s, versus 0.1441 s in component materialization.
Quantity output conversion costs 0.1649/0.1671 s; its issues section, which also
constructs the projection Python value, costs 0.2417/0.2488 s. Finalized input costs
0.2517/0.2671 s and composition 0.7224/0.7681 s. These nested spans overlap their stage
totals and are not additive independent budgets.

The next native-component investigation should start in
`components::commitments::native_context`: it constructs full observations across the
selected book before component checks. Any cached or narrower representation must
preserve the provenance-filtered authoring view, all row validation and refusal
ordering; observer authority cannot be reused as authoring evidence. Further work should
target shared native-component facts and canonical commitments, then remove
quantity/finalized re-materialization within custody. The separately owned fusion and
adapter work must be integrated before whole-pipeline acceptance.
