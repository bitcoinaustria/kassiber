# Custody 10× plan (R4, plan 21)

Local branch `t3code/custody-10x`. It starts at `8355878af` (components
checkpoint) and merges the evidence line (`3b9f0b0f1`) and the projection line
(`1f9763c3b`). The goal is Rust custody CPU at least 10× below Python on all
four bench books, with byte-identical stage output. This file is the plan
written before the structural changes; measured results are appended as the
steps land.

## Starting point

Single run on the merged tree, shared host (one-minute load 30–90), process CPU
clock for native spans, `process_time` for Python. Python's custody total is
the sum of its six stage spans and excludes its own snapshot load and
admission. Native custody (`custody.total`) includes admission.

| Book | Python custody | Native custody | Speedup | 10× budget |
| --- | ---: | ---: | ---: | ---: |
| generic 2k | 0.852 | 0.318 | 2.7× | 0.085 |
| multi-wallet 2k | 1.181 | 0.531 | 2.2× | 0.118 |
| generic 20k | 7.103 | 3.173 | 2.2× | 0.61–0.71 |
| multi-wallet 20k | 16.251 | 5.031 | 3.2× | 1.44–1.63 |

Python times vary with load, so the 20k budget is a range. The lower bound uses
the quieter reference run in `performance-components.md` (6.10 s and 14.35 s).

Native stage CPU (seconds), generic 20k / multi-wallet 20k: admission
0.45 / 0.60, components 0.00 / 0.81, evidence 0.87 / 0.95, interpretation
0.22 / 0.69, gaps 0.16 / 0.18, quantity 0.62 / 0.85, finalized projection
0.79 / 0.86.

## Profile

A local `perf_event_open` task-clock sampler records frame-pointer
callchains, using one event per CPU on a child process it starts. ptrace
scope 1 blocks attaching to other processes. It runs on a separate
frame-pointer release build and symbolizes with `llvm-symbolizer`, including
inlined frames. The tool and raw samples are in `target/c10x/`.

Self time inside custody, generic 20k:

- string comparison in ordered-map lookups, which is most of the
  `serde_json::Map` and `Record` field access: 19%;
- allocation, copy and free from cloning rows, observations and stage
  objects: about 20%;
- the loaded snapshot's JSON round trip for admission (`StageSnapshot::typed`
  serializes 68 MB and parses it back): 9%;
- conversions between `serde_json::Value` and `kassiber_pyjson::Value`
  (`borrowed_py`, `from_json`, `to_json`, `lossless`): about 12%;
- UTF-8 validation, Python text helpers (`py_lower`, `strip`, `in_ranges`)
  and decimal formatting make up the rest.

By stage, the inclusive shares are: evidence 27/20%
(`enrich_with_keys`, `canonical_event`, `event_key`, receipt scopes),
finalized projection 26/15% (`evaluate_with_rows`, row overlays,
`records`), quantity 19/17% (`compose`, `arbitrate`, `postings`,
`borrowed_py`), interpretation 8/17% (`safety::compose` over observation
Values), gaps 6/4% (worklist rows cloned into Values, then normalized),
channel admission 3% (computed twice), memo teardown 2%. Multi-wallet adds
components at 18%, of which `native_context` is 15%. It rebuilds enriched rows
and event keys for every transaction that evidence builds again.

The 2k books show the same shape; gaps rise to 15% because the small-book
worklist runs the full search.

### What sets the floor

Python's stage outputs are large. At generic 20k the response is 308 MB:
evidence 89 MB (canonical input 67 MB), quantity 130 MB (the canonical input
again, plus 59 MB of projection), finalized rows 50 MB. Each stage currently
materializes its output as a `Value` tree, and the next stage reads it back
through string-keyed lookups. Building and walking about 270 MB of `Value`
trees costs more than the whole 10× budget. No local optimization inside a
stage removes that, which is why one-stage-at-a-time typing gained only
10–30%.

## Design

One typed model from loader rows to finalized rows. JSON appears only where a
caller serializes a stage.

1. **Rows.** The loader and the v1 parser both produce typed transaction
   rows: fixed column slots, interned identifier columns (`id`, `wallet_id`,
   `profile_id`, `asset`, `journal_transaction_id`) and borrowed text. Field
   access is a slot read, not a key search. Admission runs over these rows
   directly; the `typed()` JSON round trip is removed.
2. **Interning.** Transaction ids, wallet ids, assets and quantity hashes
   become `u32` symbols in one per-run interner. Hash-keyed indexes replace
   `BTreeMap<String, _>` lookups, except where Python's iteration order
   matters; those keep an ordered index keyed by the resolved string.
3. **Evidence.** It produces an arena of typed `Observation`s (`ObsId`
   indexes): commitment, authority, event key and quantity facts. It also
   produces typed `CanonicalEvent`s holding leg, alias and snapshot indexes,
   plus typed enriched-row views that borrow the row and store only their
   overrides. Each commitment preimage is written once into the payload, and
   its SHA-256 is computed once. Components' native context reuses the same
   observations instead of rebuilding enriched rows and keys.
4. **Interpretation, gaps and quantity** read observations by `ObsId`
   through typed accessors. Claims, decisions and postings are typed records
   that reference observations by id, not copies. The quantity
   `canonical_input` and `projection.observations` outputs become views of
   the evidence arena, not clones.
5. **Finalized projection** emits typed row overlays: the source row
   reference plus generated fields (`id`, `custody_*`, slice bounds). It does
   not copy whole rows.
6. **Lazy JSON.** Each typed stage can write its exact Python JSON (same key
   order, same number spellings) and, while its consumers are migrated, build
   the identical `Value` lazily and once. Unported, rarely exercised
   interpreter paths (channels, HTLC, Samourai, loans, conversions) keep their
   current `Value` code and read that lazy view, so their behaviour is
   unchanged. The lazy view is built only on books that exercise them.
7. **Output.** `Stages` carries the typed stages. The binding response and
   the journals render input serialize from them. Render input reads the typed
   finalized rows and quantity state directly. If a `Value` is still needed,
   it is converted from typed data, never through the `pyjson` round trip it
   uses today. Render input and complete-process CPU are reported next to
   custody CPU, so no cost moves out of the measured span unnoticed.
8. **Hashing.** Canonical hashes are computed only where the output contains
   them or where identity depends on them. A hash that only a skipped
   diagnostic would contain is deferred.
9. **Parallelism.** The metric is process CPU, so threads don't help.
   `par::map` already runs with one worker. No new parallel sections are
   added; any later one must preserve order and show a CPU gain.

Python, the v1 snapshot contract, the binding signature and response bytes,
and the journals process contract stay unchanged.

## Per-stage budget (seconds, 20k)

| Stage | Generic budget | Multi-wallet budget | Main change |
| --- | ---: | ---: | --- |
| Admission | 0.04 | 0.06 | typed rows, no JSON round trip, channel admission computed once |
| Components | 0.00 | 0.25 | native context reuses evidence observations; typed legs |
| Evidence | 0.20 | 0.25 | typed observation arena, single commitment write, no enriched-row copies |
| Interpretation | 0.06 | 0.30 | typed observation access in safety/holds/pipeline; lazy `Value` for rare interpreters |
| Gaps | 0.05 | 0.06 | worklist normalization straight from typed rows |
| Quantity | 0.12 | 0.30 | typed claims, arbitration and postings over `ObsId`; no `borrowed_py` |
| Finalized projection | 0.12 | 0.15 | typed row overlays; no `from_json`/`to_json`/`lossless` passes |
| **Total** | **0.59** | **1.37** | |

2k books follow the same split: generic ≤ 0.085 s, multi-wallet ≤ 0.118 s.
The fixed per-run costs, such as memo setup and teardown, channel context and
gap search limits, have to fit that budget too.

## Steps

Each step is committed only after this gate passes: Rust custody tests
including the frozen vectors, fmt and clippy, the 24,000 generated books plus
1,030 permutations, the 159 recordings plus permutations, and the touched-file
mutations re-anchored. Each commit records its numbers.

1. Typed admission over loaded rows, with no snapshot JSON round trip, and
   channel context computed once.
2. Typed rows and interner shared by the loader, the v1 parser and stages.
3. Evidence observation arena with lazy views; components' native context
   reuses it.
4. Gaps normalization from typed rows.
5. Quantity over observation ids: typed claims, arbitration, postings and
   decisions.
6. Finalized projection overlays.
7. Interpretation hot paths (safety, holds, pipeline, privacy) on typed
   observations.
8. Typed `Stages`, lazy JSON, and render input reading the typed stages.
9. Full mutation catalog in resumable shards, end-to-end `journals process`
   against Python on all four books, and the final best-of-five alternating
   report.

If a stage cannot reach its budget under byte parity, the report will name
the floor, show the profile evidence for it and give its cost.

## Results so far (2026-11-03)

Steps 1–14 are committed on this branch; each one passed the gate above
before the next started. Mutations ran on the changed hunks of each step,
and the full catalog is still to run.

Best of five alternating Python/native pairs on step 13 (`0ee0f2383`),
custody CPU in seconds, with the one-minute load during the run:

| Book | Python | Native | Speedup | Load |
| --- | ---: | ---: | ---: | --- |
| generic 2k | 0.763 | 0.085 | 9.0× | 17–18 |
| multi-wallet 2k | 1.091 | 0.141 | 7.7× | 13–16 |
| generic 20k | 5.181 | 0.785 | 6.6× | 5–12 |
| multi-wallet 20k | 12.397 | 1.311 | 9.5× | 6–18 |

The single-run Python figures in "Starting point" overstated Python by up to
37% (generic 20k: 7.10 s there, 5.18 s here). Only alternating pairs are
comparable. Step 14 (record column positions) took native custody to 0.081 /
0.141 / 0.739 / 1.228 s in a best-of-three A/B. Against the pairs above that
is 9.4× / 7.7× / 7.0× / 10.1×.

What the steps changed, by cost removed:

- JSON rows are no longer materialized for custody. Interpretation, evidence,
  derivation and finalization read snapshot records through one `Row`
  trait, and derivation layers its overrides over them.
- Each raw JSON text is loaded once per operation. Single fields are read
  from the loaded value, and the transfer decoding is built only when a
  reader needs the whole object.
- Record field reads remember column positions; a remembered position is
  checked against its name.
- Large JSON parts move into their results instead of being copied by
  `json!`, as do pairs, components and claim lists.
- Default arbitrated decisions are read without building them.
- Finalized rows overlay generated fields on their source records.
- Gap search ranks qualifying groups by keys it already holds.

### Where the remaining time goes (step 14)

Instructions retired in native custody, generic 20k, step 14 (3.51 G in
total), by stage: evidence canonical events 28%, finalized projection 15%,
gap admission and search 15%, evidence enrichment 11%, quantity 11%,
interpretation 9%, admission 8% (Python's figure has no admission). glibc
allocation and string comparison together were 35%.

## Results (2026-11-05)

Steps 15–28 took the four structural changes listed at step 14 and the
hot paths they exposed:

1. Finalized rows hold typed overrides (integers, statics, references to
   observation legs). Known field names carry key-order ranks kept inline,
   fields are appended and sorted only when a row is written, and an
   override entry is 56 bytes.
2. Evidence events keep typed snapshots, aliases and keys: a selected
   observation moves into its leg, snapshots and aliases name legs, key
   objects are built on request, and events group by sorting. Observation
   quantities stay machine integers unless they exceed one.
3. Admission reads the loaded network JSON in place, types an observation
   only when its columns are new, and types rate rows only when the gate
   reads them.
4. Interpretation and quantity resolve aliases through their legs, read
   typed event keys, and check principals and quantity rules on machine
   integers.

Step 21 also keeps evidence amounts exact beyond i128 (the overflow the
integration branch fixed in 3defac865 existed in the typed observations
too).

Best of five alternating Python/native pairs on step 28 (`9f25c45cb`),
custody CPU in seconds:

| Book | Python | Native | Speedup | Load |
| --- | ---: | ---: | ---: | --- |
| generic 2k | 0.813 | 0.060 | 13.6× | 16–17 |
| multi-wallet 2k | 1.159 | 0.113 | 10.3× | 15–16 |
| generic 20k | 5.188 | 0.519 | 10.0× | 10–14 |
| multi-wallet 20k | 13.211 | 1.005 | 13.1× | 8–12 |

Per stage, generic 20k (Python / native): evidence 1.778 / 0.143,
interpretation 0.947 / 0.049, gaps 1.107 / 0.061, quantity 0.670 / 0.073,
finalized projection 0.444 / 0.102; native admission adds 0.016.
Finalized projection (4.4×) is still the weakest stage. Under machine load
of 60–150 the same binaries measured 3–5% slower, so the 10.0× and 10.3×
figures have no margin.

## End to end (2026-11-05)

Native `journals process` CPU time by phase, step 28 binary with drop
spans added for this measurement only, best of three, quiet machine
(seconds). Python figures are the process CPU of the alternating pairs
above. "Drops" is freeing the phase results when `process` returns;
"other" is process start, request parsing, repair, pricing and the
remaining unspanned work.

| Phase | generic 2k | multi-wallet 2k | generic 20k | multi-wallet 20k |
| --- | ---: | ---: | ---: | ---: |
| snapshot load | 0.027 | 0.049 | 0.277 | 0.388 |
| custody | 0.062 | 0.110 | 0.513 | 0.978 |
| render inputs | 0.062 | 0.068 | 0.634 | 0.687 |
| fusion engine | 0.141 | 0.168 | 1.205 | 1.535 |
| persistence and commit | 0.074 | 0.082 | 1.187 | 1.273 |
| balance comparison | 0.001 | 0.002 | 0.014 | 0.016 |
| drops | 0.040 | 0.048 | 0.391 | 0.428 |
| other | 0.039 | 0.038 | 0.349 | 0.370 |
| **native process** | **0.446** | **0.563** | **4.570** | **5.674** |
| Python process | 1.75 | 2.17 | 12.46 | 21.87 |
| speedup | 3.9× | 3.9× | 2.7× | 3.9× |
| native budget for 4× | 0.438 | 0.543 | 3.115 | 5.468 |

Generic 20k is 1.46 s over its 4× budget; the other books are 0.01–0.21 s
over. Where the generic 20k phases go (CPU profile of the whole process):

- Fusion engine, 1.205 s: building the ledger request from the rendered
  projection 0.31 s (`ledger_request::build_request`, mostly JSON map
  lookups in each rendered row), writing it as text 0.10 s, the tax engine
  parsing that text and computing 0.25 s, loading and converting the
  response text 0.17 s, freeing request and response 0.14 s, and
  `boundary::ledger_response` 0.06 s. The request and response cross a
  JSON text boundary between two Rust crates.
- Persistence, 1.187 s: `persistence::store_ledger_state` writing journal
  entries, quantities and calculations through SQLite.
- Render inputs, 0.634 s, plus 0.249 s freeing its JSON when `process`
  returns: `render_inputs` builds the rendered quantity, projection and
  entry documents as JSON trees that the fusion request and
  `boundary::build` then read by key.
- Snapshot load, 0.277 s: the direct loader's row reads and record building.

Ownership: request building (`ledger_request`) is custody-crate code and in
this branch's scope. The tax engine API (a typed request instead of text),
journal persistence, `render_inputs`, `boundary::build` and the direct
loader belong to the journals and tax-core crates. Reaching 4× on generic
20k needs about 1.5 s from them together; persistence, the text boundary
and the rendered JSON trees are the largest parts.

## Margin work (steps 29–33, 2026-11-05)

Steps 29–33 trust typed observations in arbitration (checked in debug
builds), balance domains in i128, write presorted dicts without sorting,
and move claim, quarantine and coverage lists into their results instead
of letting `json!` copy them. Best of five alternating pairs on step 33
(`de7434100`), custody CPU in seconds:

| Book | Python | Native | Speedup | Load |
| --- | ---: | ---: | ---: | --- |
| generic 2k | 0.782 | 0.062 | 12.5× | 18–20 |
| multi-wallet 2k | 1.191 | 0.120 | 9.9× | 16–17 |
| generic 20k | 5.444 | 0.505 | 10.8× | 9–15 |
| multi-wallet 20k | 13.296 | 0.974 | 13.7× | 8–9 |

Multi-wallet 2k measured 0.113 s at step 28 and 0.120 s here, although
an A/B in one sitting puts step 33 3% below step 28 (0.118 against
0.122 s) and its instruction count 6% below. At 2k the custody total is
about 0.1 s, and the machine's state moves it by ±6% between sittings, so
that book needs about 10% more before 11× holds in every sitting. Its
largest remaining parts are component materialization (30% of its custody
instructions: per-component evidence status, anchor and conservation
checks, leg projection), component claim compilation (9%) and gap search
(7%).

End to end at step 33 (process CPU, same pairs): generic 2k 1.75 / 0.45 s
(3.9×), multi-wallet 2k 2.25 / 0.60 s (3.75×), generic 20k 13.06 / 4.68 s
(2.8×), multi-wallet 20k 22.11 / 5.90 s (3.75×). Changes inside this
branch to the ledger request (`build_request`, `request_json`) did not
move the fusion phase measurably and were not kept; the phase table above
still describes where the time goes.
