# Integrated custody performance checkpoint

Local branch `t3code/custody-perf-4`, based on `925800ebe`. This checkpoint
retains the existing custody stage contracts. The **10× Python target remains
open**, including the 0.51 s generic and 1.21 s multi-wallet design budgets.

The changes borrow component evidence rows through a provenance-filtering
view, write canonical hash preimages from borrowed JSON operands, convert
machine-sized integers without decimal text, and borrow finalized source rows
until a selected row must be copied. Unsupported hash operands retain the
original Python-value fallback. Custody maps use the calling thread: the prior
worker allocation costs and repeated thread-local memo work outweighed their
CPU savings in this integrated path. Refusal ordering and output ordering stay
unchanged.

## Method and profiling

The two committed-seed 20,000-transaction books run through the unregistered
R4-13 prepared process harness. Native pre-build resolution/repair/pricing
(R4-10) and the registered CLI/daemon shell remain outside this measurement.
Rust is 1.99.1, Python is the locked 3.11 environment, Cargo is offline with
eight build jobs, and the native binaries are release builds with debug=1.
There are no new dependencies.

Each comparison contains five alternating pairs on fresh copies of the same
migrated book. Process CPU is user plus system time across all threads, read
with `getrusage(RUSAGE_CHILDREN)`. A scratch-only preload maps the native
`Instant` spans to `CLOCK_PROCESS_CPUTIME_ID`; Python's existing phase profiler
uses `process_time`. No clock instrumentation enters the crate or binding.
The tables retain all spans from each engine's fastest complete process trial;
they do not combine independent per-stage minima. Nested spans overlap.
The custody span ends when the stages return; retained stage-output destruction
after that boundary is included in the complete-process CPU figure.

The `perf` executable is absent, but Linux perf events are available. A local
standard-library sampler uses `perf_event_open`, a 1 ms software task-clock
period, CPU-scoped rings, and thread inheritance on the saved integrated native
baseline. It captures 21,187 generic samples across 161 threads and 23,078
multi-wallet samples across 249 threads, with zero lost records in either run.
Kernel work is excluded. Frame-pointer callchains are incomplete, and stripped
system-library locations remain unresolved instead of using misleading nearest
export names. Profiler overhead is excluded from the paired performance tables.

The local `pmp.py` GDB sampler captured another 382 generic and 500 multi-wallet
integrated baseline stacks. The profiles identify row cloning,
allocation/deallocation, worker maps and finalized composition as useful kernel
targets. Main-thread GDB samples do not establish per-thread CPU percentages.
CPU spans provide the stage attribution below. This checkout contains no tracked
`pmp.py`; the saved local sampler is retained with the other raw evidence.

## CPU results

Seconds, Python/native comparison:

| Stage | Generic Python | Generic native | Multi-wallet Python | Multi-wallet native |
| --- | ---: | ---: | ---: | ---: |
| Admission | included below | 0.4941 | included below | 0.5767 |
| Components and migration | 0.2766 | 0.00004 | 2.4146 | 1.4761 |
| Evidence | 2.0500 | 1.5031 | 2.1421 | 1.4970 |
| Interpretation | 1.0431 | 0.2639 | 3.0880 | 0.7621 |
| Gaps | 1.3190 | 0.2074 | 1.2026 | 0.2261 |
| Quantity | 0.8699 | 0.6649 | 3.9214 | 0.8064 |
| Finalized projection | 0.4895 | 1.1153 | 0.5101 | 1.2439 |
| **Custody total** | **6.0482** | **4.3081** | **13.2790** | **6.6745** |
| **Complete process** | **15.5441** | **12.5356** | **22.9676** | **15.4422** |

Python admission is included in components. Custody speedups are 1.40× and
1.99×; complete-process speedups are 1.24× and 1.49×. Median custody CPU is
6.1516/4.5682 s generic and 14.7211/6.6817 s multi-wallet. One-minute load
ranges are 75.19–175.08 and 131.77–192.38, respectively. Wall spans are retained
in the machine-readable record, but this shared-host run does not meet the
design's idle-host condition.

Independent saved-native-baseline/candidate comparisons:

| Stage | Generic before | Generic after | Multi-wallet before | Multi-wallet after |
| --- | ---: | ---: | ---: | ---: |
| Admission | 0.9710 | 0.4916 | 1.0251 | 0.6285 |
| Components and migration | 0.00004 | 0.00003 | 2.7035 | 1.4486 |
| Evidence | 2.3449 | 1.3844 | 1.9882 | 1.4882 |
| Interpretation | 0.2548 | 0.2360 | 0.8025 | 0.7467 |
| Gaps | 0.3241 | 0.1817 | 0.3343 | 0.2094 |
| Quantity | 0.7058 | 0.6487 | 0.9023 | 0.7788 |
| Finalized projection | 1.8499 | 1.1555 | 1.7612 | 1.1983 |
| **Custody total** | **6.4831** | **4.1538** | **9.5811** | **6.6011** |
| **Complete process** | **14.2085** | **12.4678** | **19.1599** | **15.1832** |

Custody gains are 1.56×/1.45×; complete-process gains are 1.14×/1.26×. Load
ranges are 134.31–192.33 generic and 112.07–175.81 multi-wallet. All twenty
paired full scalar-exact database dumps, payloads and phase sequences agree.

## Verification and continuation

The final kernel batch passes all twelve generators: 24,000 books and 1,030
permutations, with 50,060 byte comparisons and 178 matching refusals. All 159
recordings and their 159 permutations pass (636 byte comparisons). All 621
Rust custody tests, including frozen vectors, pass; formatting and clippy with
warnings denied pass. The catalog retains all 2,327 inherited recipes, reanchors
seven row recipes and the exception-family recipe, and adds six borrowed-row/hash
recipes, for 2,333 total. The old exception-family mutant merely removed a
redundant guard after shared integer admission began preserving `TypeError`;
its replacement directly corrupts that same error family, with the original
rule and frozen test retained.
Mutation, classification and preflight results are recorded in
[`performance-4.json`](performance-4.json).

The machine-readable record retains every paired trial, stage CPU, process CPU
and wall time, load observations, binary hashes, changed-source hashes, suite
counts, and catalog audit. Local raw profiles, generated scenario activity and
verification logs are under `target/custody-perf-4/`.

Remaining work is substantial: evidence still builds and copies large JSON
graphs, admission retains transitional row materialization, and quantity and
finalized projection retain conversions and owned output. The finalized stage
still costs over twice Python. These costs explain the budget misses; this is
not R4-3/R4-7 acceptance. The separately owned renderer/fusion-conversion and
persistence work must be integrated before repeating whole-pipeline acceptance.
