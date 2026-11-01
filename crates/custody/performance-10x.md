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
