# R4 journal pipeline design

> **Historical.** This page records the Python-to-Rust port. Python was removed
> under [plan 22](../plan/22-python-removal.md); the `kassiber/` paths and Python
> tests it cites exist only at the `archive/python-backend` tag.

**Status:** R4-0 round 4 integrated, 2026-10-31; the R4-1 bench harness and
four recovered baseline records are committed. The typed row and admission
continuation is an **R4-2 checkpoint**, with acceptance still open; R4-3's
components/evidence rewrite is not implemented. The subsequent
[interpretation and gap kernel checkpoint](custody-engine.md#interpretation-and-gap-kernel-checkpoint-r4)
meets both stage CPU targets while retaining the existing representations. The R4-7 typed finalized-output probe also missed its targets and was withdrawn
([probe](#r4-7-finalized-projection-target-probe)). The R4-11 renderer has a
partial checkpoint; production acceptance
remains open. A subsequent [projection and fusion kernel checkpoint](#projection-and-fusion-kernel-checkpoint)
keeps the existing stage models; its performance targets remain open.
[Storage-format speedups](#storage-format-speedups) run Python
first; compact calculations are implemented with the Rust schema mirror.
The R7 integration audit records the linear
restack and its verification. The separate [tax-engine speed pass](tax-engine.md#ledger-performance-checkpoint-r4-2026-10-30)
measures 2.03–3.96× faster `ledger` calls with byte-identical responses; its
timings supersede the engine line in this design's original measurements.
R4-9 has a test-only loader checkpoint
whose accepted determinism continuation combines loader-9's exhaustive map,
lossless BLOB consumers and one exact typed revision refusal. All forty older
seed-dependent message exceptions are closed in that source record, inherited
from t3code/r4-9-loader-determinism@aaf6216e3. Fresh R9 verification passes
274 loader/BLOB cases, generated and archived custody/render campaigns, and
focused census/shadow; the R9 audit
tracks the remaining combined gates. Historical timings keep their source scope.
Python retains production authority.
The [R4-14 pairing stage](rust-pairing.md) reuses the native transfer library;
its 20k process CPU acceptance is 13.02× generic and 11.75× multi-wallet.
Journal route activation remains with R4-13/R4-15/R4-16.
Those CPU measurements are inherited from t3code/r4-14-auto-pair@c47450757.
The R9 workspace passes the integrated pairing stage tests; combined CPU and
mutation results are tracked in the R9 audit.

R4-9's test-only loader acceptance
closes its exhaustive branch map, 308 loader and 2,292 custody mutations,
and stage generated/recorded gates. The BLOB and deterministic-refusal
continuations close the storage gaps and all forty former message exceptions.
The loader is fully accepted. These are stage records; merged full-process
gates remain R4-13 work.
R4-13 has a test-only composition checkpoint:
real ordered R4-10 repairs/pricing through native load and persistence with
transactional rollback and progress, and no CLI or daemon registration. Its
test-only probe does not close whole-pipeline parity or performance acceptance.

that misses its 0.30 s target and remains unaccepted and unrouted.
R4-10 has [functional acceptance](#r4-10-functional-acceptance) for its native
pre-build boundary, with the scoped-cache multi-wallet CPU miss recorded.
Production integration belongs to R4-13.

test-only refresh coordinator also owns the R4-14 pairing commit and rollback,
retaining a completed stale rebuild across a failed second rebuild. The probe
does not close whole-pipeline parity or performance acceptance.
The [R4-14 pairing stage](rust-pairing.md) reuses the native transfer library;
its 20k process CPU acceptance is 13.02× generic and 11.75× multi-wallet.
Journal route activation remains with R4-13/R4-15/R4-16.
This document plans stage R4 of

R4-9's accepted loader-9 and BLOB continuations are integrated; **40 measured
seed-dependent messages remain pending the Python fix**. Its
test-only loader acceptance
records the exhaustive map, complete mutation campaigns and fresh parity gates.
Loader-8's audit and timing evidence are unchanged; Python retains production
load and journal authority. This document plans stage R4 of
[plan 21](../plan/21-rust-backend.md): `journals process` end to end in the Rust
binary, byte-identical to Python and much faster. It builds on the
[custody engine](custody-engine.md) (R2, including the R2g fusion boundary and
the performance checkpoint) and on the storage and service-slice work of the
R3/R5 stack.

Two initial measurements shaped the design; the later probes refine the
implementation approach:

- Python spends most of its time in custody and in the Python halves of the
  ledger path. Rust custody behind the JSON binding is barely faster than
  Python on generic books. The initial assumption that typed stage records
  alone would supply the speedup did not survive three probes: local gains
  of 10–30% were consumed by conversions to untyped neighboring stages.
  Profile-driven kernels now keep each stage's existing boundaries and use
  borrowed or indexed internal structures where measurements justify them.
  The R4-4/R4-5 kernel pass reduces interpretation CPU by 3.05–4.12× and
  gaps by 2.02–2.16×; broader custody acceptance remains open.
- SQLite writes have a floor of about 1.25–1.7 s for a 20,000-transaction
  plaintext book and about 3 s for a SQLCipher one, whatever language issues
  them. Under byte parity, end-to-end `journals process` therefore cannot
  reach 10× at 20,000 transactions. The compute pipeline can, and this design
  sets 10× there.

## R4-12 persistence CPU budgets

Owner decision for this continuation: unchanged cells must complete within the
borrowed retained-row read floor plus **0.15 process CPU seconds**. Fresh cells
must complete within the prepared DML write floor plus **0.15 process CPU
seconds**. Use independent minima from at least five runs with recorded host
load. Repeat the independent-connection read floor in the quietest available
host window; a busy-host minimum must retain that qualification. Complete dumps,
signed-zero handling and the remaining mutation, campaign and caller-contract
gates remain required; a budget decision alone does not grant acceptance.

## Measured phase profile

This profile describes the pipeline before the storage-format speedups. The
implementation measurements below record the compact and changed-row writers.

### Method

- Python: [`scripts/benchmark-journals-phases.py`](../../scripts/benchmark-journals-phases.py)
  runs the real CLI entry point (`kassiber --machine journals process`) in a
  fresh interpreter on a fresh copy of the book, with wall-clock wrappers
  around each phase. The custody stage spans come from the build observer.
  Best of three runs at 2,000 transactions and best of two at 20,000; medians
  were within 2% of the best run.
- Rust custody: the same script times `kassiber_tax.custody` on the very
  snapshot the Python run loaded. `custody-bench` (`--features timing`) gives
  the in-process stage spans on that snapshot.
- Rust fusion: the R2g request builder and the tax engine on `t3code/r2g-fusion-6`,
  timed from that snapshot's Rust projection by a scratch example that times
  `build_request`, `request_json`, the engine's typed request parse and
  `ledger` separately. It is not committed, because it needs that branch's
  `ledger_request` module.
- SQL floors: [`scripts/benchmark-journals-sql-floor.sh`](../../scripts/benchmark-journals-sql-floor.sh)
  runs the snapshot's largest reads and the full replacement of every stored
  journal table in the `sqlite3` shell (SQLite 3.54) with the pragmas
  `kassiber/db.py` applies to plaintext WAL books. The SQLCipher floor used
  `sqlcipher3` (SQLite 3.51.1, DELETE journal, `synchronous=FULL`, as Kassiber
  configures it), timing statements only.
- Machine: Linux x86_64, 48 cores, Python 3.11, Rust 1.99.1 release builds.
  The load average stayed between 0.9 and 2.9 because another tenant was
  intermittently active. Python is single-threaded; Rust custody uses up to
  eight threads.

The generic books were created with this branch's Python CLI from the
`bench.py` generator's CSVs. The multi-wallet and barrier books are the
performance checkpoint's bench books. Those generators lived in the local
verification tooling; R4-1 commits the generic and multi-wallet ones as
[`scripts/journals-bench-books.py`](../../scripts/journals-bench-books.py),
which writes byte-identical CSVs:

| Book | Shape |
| --- | --- |
| generic 2,000 / 20,000 | one custom BTC wallet, buys and sells (`bench.py`'s generator, seed 1) |
| multi-wallet 2,000 / 20,000 | four wallets on Bitcoin and Liquid, same-txid transfers, 171 / 300 authored peg pairs (`rich_book.py`) |
| barrier 2,000 | Liquid barrier book: 1,985 of 2,001 transactions quarantined |

The earlier notes of about 6 s at 2,000 and 24 s at 20,000 transactions were
not reproduced. They came from another checkout's Python on a loaded machine.

### Python `journals process`, seconds

| Phase | generic 2k | multi-wallet 2k | barrier 2k | generic 20k | multi-wallet 20k |
| --- | ---: | ---: | ---: | ---: | ---: |
| Interpreter start and imports | 0.249 | 0.238 | 0.235 | 0.256 | 0.239 |
| Argument parsing, book open, scope | 0.098 | 0.111 | 0.115 | 0.152 | 0.216 |
| Pre-build repairs and auto-pricing | 0.002 | 0.003 | 0.003 | 0.025 | 0.033 |
| Snapshot load (SQL and row sanitizing) | 0.144 | 0.193 | 0.194 | 1.570 | 1.886 |
| Custody: components (with row capture, rates, channel context) | 0.024 | 0.276 | 0.232 | 0.226 | 2.207 |
| Custody: evidence | 0.170 | 0.180 | 0.195 | 1.779 | 2.006 |
| Custody: interpretation | 0.079 | 0.299 | 1.172 | 0.940 | 3.013 |
| Custody: gap search | 0.445 | 0.233 | 0.493 | 1.104 | 1.176 |
| Custody: quantity | 0.033 | 0.094 | 0.062 | 0.699 | 3.261 |
| Custody: finalized projection | 0.054 | 0.028 | 0.025 | 0.381 | 0.405 |
| **Custody total** | **0.805** | **1.110** | **2.180** | **5.131** | **12.069** |
| Ledger request build (Python) | 0.108 | 0.114 | 0.001 | 1.162 | 1.158 |
| Request `json.dumps` | 0.014 | 0.014 | 0.000 | 0.134 | 0.123 |
| Tax engine (Rust, text in and out) | 0.065 | 0.158 | 0.001 | 0.677 | 1.605 |
| Response `json.loads` | 0.008 | 0.009 | 0.000 | 0.270 | 0.304 |
| Response rendering (entries, quarantines, holdings) | 0.081 | 0.081 | 0.001 | 1.083 | 1.109 |
| Balance comparison and ownership-review joins | 0.002 | 0.002 | 0.034 | 0.033 | 0.293 |
| Persistence (`store_ledger_state`) | 0.110 | 0.128 | 0.070 | 1.616 | 1.963 |
| of which quantity-state tables | 0.030 | 0.047 | 0.043 | 0.533 | 0.736 |
| Commit and savepoint release | 0.103 | 0.130 | 0.074 | 0.632 | 0.583 |
| **End to end** | **1.797** | **2.302** | **2.910** | **12.810** | **21.653** |

- Freshness and the journal job: `journals process` evaluates no freshness.
  The freshness reads in the `--auto-pair` step below take at most 0.01 s.
- Rendering is almost all per-entry work: 1.04 s of the 1.08 s at 20,000.
- The snapshot load at 20,000 splits into the transactions query and row
  dicts (0.54 s), `require_book_accounting` (0.31 s), row sanitizing
  (0.23 s), network facts (0.14 s), the gap worklist (0.13 s), receipt groups
  (0.13 s) and rate rows (0.03 s).

The desktop's "Process journals" button, the assistant's tool and the freshness
journal job run `refresh_journals_step` with automatic pairing, which is
`journals process --auto-pair`. The transfer matcher runs four times: two
candidate suggestions, rules and bulk exact pairing. One run each:

| `--auto-pair`, seconds | generic 2k | multi-wallet 2k | generic 20k | multi-wallet 20k |
| --- | ---: | ---: | ---: | ---: |
| Matcher (suggest ×2, rules, bulk) | 0.31 | 0.41 | 3.44 | 4.74 |
| End to end | 2.15 | 2.72 | 16.17 | 26.33 |

### Rust where it exists, seconds

| Phase | generic 2k | multi-wallet 2k | barrier 2k | generic 20k | multi-wallet 20k |
| --- | ---: | ---: | ---: | ---: | ---: |
| Binary start (`commands describe`) | 0.003 | | | | |
| Book open and `status` (R5 binary) | 0.100 | | | 0.142 | |
| Snapshot size / custody response size, MB | 6.8 / 27 | 9.2 / 29 | 8.4 / 25 | 68 / 270 | 77 / 282 |
| Custody through the binding, text in and out | 0.560 | 0.898 | 0.932 | 5.018 | 7.390 |
| Python `json.dumps` of the snapshot (binding caller) | 0.037 | 0.052 | 0.043 | 0.333 | 0.385 |
| Python `json.loads` of the response (binding caller) | 0.154 | 0.142 | 0.130 | 1.596 | 1.829 |
| R2g request builder (`Value`) | 0.126 | 0.123 | | 1.31 | 1.39–1.46 |
| Request wire text (`request_json`) | 0.017 | 0.017 | | 0.19 | 0.19 |
| Tax engine, text in and out | 0.057 | 0.151 | | 0.64 | 1.58 |
| of which typed request parse | | | | 0.038 | 0.039 |

`custody-bench` spans inside custody, from request text:

| Span, seconds | generic 2k | multi-wallet 2k | barrier 2k | generic 20k | multi-wallet 20k |
| --- | ---: | ---: | ---: | ---: | ---: |
| JSON parse into `Snapshot` (`Map<String, Value>` rows) | 0.042 | 0.055 | 0.050 | 0.702 | 0.658 |
| Admission (accounting, confirmation capture, rates, channels) | 0.035 | 0.035 | 0.038 | 0.274 | 0.362 |
| Components and migration quarantines | 0.000 | 0.155 | 0.113 | 0.000 | 0.763 |
| Evidence | 0.039 | 0.043 | 0.049 | 0.327 | 0.389 |
| Interpretation | 0.036 | 0.161 | 0.269 | 0.460 | 1.420 |
| Gap search | 0.086 | 0.056 | 0.091 | 0.362 | 0.380 |
| Quantity (with 0.13–0.16 s converting to Python-model values at 20k) | 0.155 | 0.193 | 0.166 | 1.576 | 1.819 |
| Finalized projection | 0.128 | 0.135 | 0.095 | 1.053 | 1.162 |
| Response text | 0.027 | 0.026 | 0.027 | 0.172 | 0.174 |
| **Total** | **0.561** | **0.898** | **0.925** | **5.099** | **7.521** |

### SQL floors

SQLite's own cost, with no Python or Rust row handling:

| Floor, seconds | generic 2k | generic 20k | multi-wallet 20k |
| --- | ---: | ---: | ---: |
| Custody transactions query, output discarded | 0.009 | 0.089 | 0.094 |
| Rate rows query | 0.002 | 0.022 | 0.023 |
| Replace every journal table: `journal_entries` delete + insert | 0.026 | 0.631 | 0.796 |
| … `journal_quantity_postings` delete + insert | 0.017 | 0.437 | 0.429 |
| … all other journal tables | < 0.001 | < 0.001 | 0.007 |
| … commit, with the automatic WAL checkpoint | 0.112 | 0.451 | 0.461 |
| **Write floor, plaintext WAL** | **0.155** | **1.52** | **1.69** |
| All journal tables with the checkpoint deferred, three runs | | 1.25–1.61, then 0.37–0.50 to checkpoint | 1.45–1.71, then 0.28–0.55 |
| Entries and postings, SQLCipher, DELETE/FULL, three runs | | 2.74–3.08 | |

- The writes are I/O bound: system time dominates. Dropping the five
  `journal_entries` indexes and rebuilding them after the insert made the
  entries' write slower (0.85 s against 0.63 s, excluding the commit), so
  index work is not the lever.
- `calculation_json` holds 37 MB of the 52 MB `journal_entries` table at
  20,000 entries. Every rebuild replaces every row: entry ids are `uuid4` and
  `created_at` is the build time, and four of the five indexes include
  `created_at`.
- Python's persistence plus commit (2.25 s at generic 20,000) is within about
  0.7 s of the plaintext floor.

### What the profile says

- **Custody** is 40–75% of Python's time. Behind the binding, Rust custody is
  1.0× (generic 20k), 1.6× (multi-wallet 20k) and 2.3× (barrier 2k) faster than
  Python. In-process it still spends 0.66–0.70 s parsing into `Value` rows
  at 20,000 and the rest on map lookups, string allocation and conversion
  between serde and Python-model values. A Python caller of the binding pays
  another 1.9–2.2 s at 20,000 to serialize the snapshot and parse the 270 MB
  response.
- **The ledger path outside the engine** costs Python about 2.7 s at 20,000
  (request build, wire text, response parse, rendering). The R2g Rust builder
  is slower than Python's (1.31 against 1.16 s) and still writes wire text.
- **The tax engine** is already Rust and typed: 0.55 s (generic) and 1.5 s
  (multi-wallet) of compute at 20,000, plus about 0.08 s of parsing and
  serializing. It is the largest compute item a typed custody port leaves.
- **Snapshot SQL** costs Python 1.57–1.89 s at 20,000 against a 0.11 s floor
  for the two large queries.
- **Persistence** is bound by SQLite I/O, not by Python.
- **Fixed costs:** Python's interpreter and imports take 0.25 s; the Rust
  binary starts in 3 ms, but its R5 book open takes 0.10–0.14 s.
- One typed data point: the engine's typed parse reads 14 MB in 0.038 s
  (about 370 MB/s), while custody's `Value` parse reads 68 MB in 0.45–0.70 s
  (100–150 MB/s). The documents differ, so this indicates the gap; it does
  not measure it.

## Target architecture

One process, one transaction, no JSON between stages.

```text
kassiber journals process (CLI) / ui.journals.process, journal_refresh job (daemon)
  └─ crates/journals::process(book, scope, clock) ──── BEGIN IMMEDIATE
       ├─ prebuild: source-overlap repair, legacy component refresh, auto-pricing   (writes, savepoint)
       ├─ load:     SQLite ──► BookSnapshot                       (typed rows, crates/store)
       ├─ custody:  BookSnapshot ──► Components ──► Evidence ──► Interpretation
       │                     ──► GapSearch ──► QuantityState ──► FinalizedProjection
       ├─ fusion:   FinalizedProjection ──► LedgerRequest          (tax-engine typed request)
       ├─ engine:   LedgerRequest ──► LedgerResponse                (tax-engine typed response)
       ├─ render:   LedgerResponse ──► JournalState                 (entries, quarantines, holdings, joins)
       └─ persist:  JournalState ──► prepared statements, freshness markers, filed-report impacts
                    ──► RELEASE, COMMIT ──► reply ──► WAL checkpoint
```

### Placement

- `crates/journals` (new, per the service-slice contract): orchestration,
  pre-build repairs, the SQLite loader, rendering, joins and persistence. It
  depends on `crates/store`, `crates/custody` and `tax-engine/core`.
- `crates/custody` stays pure: typed snapshot in, typed stages out, no SQL,
  no clock. `custody_with_finalized(&str, bool)` remains as the JSON adapter.
- `tax-engine/core` exposes `ledger_typed(&LedgerRequest)
  -> Result<LedgerResponse, StagedError>` beside `ledger(&str)`. Both use the
  same computation. The prepared native process passes its typed response to
  the renderer; direct typed fusion and the remaining custody value adapters
  are still open in the conversion checkpoint.
  The text entry stays for Python and fixtures.
- The CLI and daemon register `journals process` and `ui.journals.process`
  through one module each; the freshness `journal_refresh` handler calls the
  same `process`.

### Typed model per stage

Each stage's type mirrors the Python dataclass or dict it replaces field for
field, keeps today's exact value types and implements `Serialize` that writes
today's oracle JSON for that stage (`$type` tags, key order, `pyjson` floats
and `Decimal` text). Shared building blocks:

- `SqlValue`: `Null | Int(i64) | Real(f64) | Text(Box<str>) | Blob(Box<[u8]>)`,
  for columns whose historical storage class varies. Columns the schema and
  compatibility pass guarantee get concrete types.
- Interned identifiers (`Arc<str>`) for transaction, wallet, asset and
  component ids, which every stage joins on.
- Amounts stay integer msat (`i64` where the column is `INTEGER`, `BigInt`
  where Python arithmetic can exceed it), decimals stay `pydecimal`, timestamps
  keep their raw text next to any parsed instant.

| Stage | Type | Replaces |
| --- | --- | --- |
| Load | `BookSnapshot { profile, book_network, wallets: Vec<WalletRef>, transactions: Vec<TxRow>, rate_rows: Vec<RateRow>, owned_index: Option<OwnedIndex>, components: Vec<AuthoredComponent>, anchors, anchor_wallets, revision_headers, receipt_sources, channel_records, loan_legs, evidence_snapshots, migration_issues, gap_reviews, pair_dismissals, gap_worklist }` | `load_custody_snapshot` dicts; `Snapshot` with shared-column `Record` and transitional JSON values |
| Components | `Vec<MaterializedComponent>`, `Vec<ComponentBlocker>`, `Vec<MigrationQuarantine>` | `components` stage `Value` |
| Evidence | `Evidence { enriched_rows: Vec<EnrichedRow>, canonical: CanonicalInput { observations: Vec<QuantityObservation>, events, issues }, native_non_event_ids }` | `evidence` stage `Value` |
| Interpretation | `InterpreterCompilation { claims: Vec<QuantityClaim>, quarantines, blocked_transaction_ids, non_event_transaction_ids, native_audits, native_component_reconciliations, … }` | `pyjson::Value` |
| Gaps | `GapSearchResult { candidates, source_blockers, ignored_ids, accounting_ignored_ids, … }` | `gaps` stage `Value` |
| Quantity | `CanonicalQuantityState { postings, balances, issues, decisions, economic_relations, tax_eligibility, reviewed_direct_payouts, report_blocked, … }` | `pyjson::Value` and its 0.13–0.16 s conversion |
| Finalized | `FinalizedTaxProjection { rows: Vec<FinalizedRow>, quarantines, cross_asset_pairs, intra_pairs, … }` | `pyjson::Value` |
| Fusion | `kassiber_tax_core::ledger::request::LedgerRequest` | R2g `Value` request and its wire text |
| Engine | `kassiber_tax_core::ledger::response::LedgerResponse` | response text |
| Render | `JournalState { entries: Vec<JournalEntryRow>, quarantines: Vec<QuarantineRow>, tax_summary, account_holdings, wallet_holdings, ownership_review, quantity_differences, gap_ignored_ids, … }` | `read_ledger`, `build()`'s dict |
| Persist | prepared `INSERT` per table, typed parameter binding | `store_ledger_state` |

Wire-text fidelity for refusals: R2g found that an amount above the engine's
`u64` range is reported with the JSON column of the request text. The typed
fusion checks each amount's range while building. Only a request that would
fail that check is serialized and handed to the text entry point, so the
diagnostic stays byte-identical and the common path never writes text.

### The JSON interface stays the parity oracle

- `kassiber_tax.custody(text)` keeps its contract: it parses the v1 snapshot
  into `BookSnapshot` (`Deserialize` lossless for every recorded shape), runs
  the typed stages and serializes each stage's oracle JSON. Census,
  custody-shadow, recorded replay, the twelve generators and the frozen
  vectors keep comparing every byte. Only test runs pay for the
  serialization.
- A round-trip gate proves the adapter: `BookSnapshot` from snapshot text and
  back writes the same bytes for every corpus request, permutation and
  recording.
- The native loader has its own byte gate: on every census and shadow book,
  and on generated database fixtures, the Rust loader's `BookSnapshot`
  serialized as v1 equals Python `load_custody_snapshot`'s
  `json.dumps(…, ensure_ascii=True, allow_nan=True)`.
- Fusion compares the typed request's wire text with Python's
  `ledger_request.dumps()` and the full ledger response, as R2g does.
- Rendering and persistence compare database state: the same starting copy is
  processed once by Python and once by Rust, and complete logical dumps must
  match. Generated `uuid4` ids map through a declared one-to-one mapping and
  `created_at` comes from the injected test clock. Nothing is masked.

### Atomicity and failure

- `BEGIN IMMEDIATE` before any read, as Python does, so a freshness writer
  cannot invalidate the read snapshot. Pre-build repairs, the build and
  persistence run under one savepoint. Any refusal or panic rolls back
  completely, and the freshness markers stay stale.
- The CLI writes its reply after `COMMIT`, then runs a passive WAL checkpoint
  with automatic checkpoints disabled on that connection. The daemon runs it
  after the response record. This moves 0.37–0.50 s off the user-visible path
  at 20,000 without changing a stored byte. SQLCipher books keep DELETE
  journaling during coexistence, so nothing changes for them.
- No disk logging. Timing spans are a compile-time feature that is off in
  release builds.

## Migration path

Each step is one worker run with one verified commit boundary. Every step
keeps the full gates: the twelve generators with per-scenario activity
(24,000 books, 1,030 permutations), recorded replay (159 requests and
permutations, plus fresh recordings), census and custody-shadow, the frozen
vectors and the mutation catalog. A typed rewrite moves mutation anchors.
Each step therefore re-anchors every recipe in the code it rewrites to an
equivalent mutant, as the performance checkpoint did, reruns the **whole**
catalog (currently 2,327 recipes), and reports kill set equals catalog. The catalog may
not shrink without a reviewed equivalence note. The separate kernel branch ran its
391 touched-file recipes from a 2,301-recipe catalog; untouched recipes are
not claimed as freshly killed there. This integration reruns the whole catalog.
Each step also records before and after timings from the bench books, back to back.

"Measured cost replaced" is what the code a step rewrites costs today, at
generic / multi-wallet 20,000 transactions. It bounds the step's gain; it is
not a prediction. Each step's target is an acceptance criterion. The initial
typed sequence stopped failed probes for re-planning. The subsequent kernel
run drops that stop rule and retains every measured optimization that keeps
byte parity; its CPU ceilings remain the stage acceptance targets.

| Step | Work | Measured cost replaced (20k) | Step target |
| --- | --- | --- | --- |
| R4-0 | Join the stacks: restack `r2g-fusion-6` (7 commits) on `r2-custody-perf`, then onto the R5 tip, whose custody stops at the R2c channel-lifecycle checkpoint. Resolve `lib.rs`/codec conflicts. | — | All gates green; hashes unchanged |
| R4-1 | Bench harness in the Rust workspace: an end-to-end `journals-bench` example with spans, committed bench-book generators, the Python profiler, a regression record format | — | Baseline recorded |
| R4-2 | Typed `TxRow`, `WalletRef`, `RateRow`, worklist rows and `SqlValue`; drop the shared `Value` row view; admission over fields | parse 0.70 / 0.66; admission 0.27 / 0.36 | parse + admission ≤ 0.25 s at generic 20k |
| R4-3 | Typed components and evidence | 0.33 / 1.15 | ≤ 1/3 of today |
| R4-4 | Interpretation kernels, unchanged stage boundaries; local shared facts and lazy indexes | 0.46 / 1.42 | ≤ 1/3 CPU; met by the kernel checkpoint |
| R4-5 | Gap normalization and beam-search kernels, unchanged stage boundaries | 0.36 / 0.38 | ≤ 1/2 CPU; met by the kernel checkpoint |
| R4-6 | Typed quantity state; remove the Python-model conversion | 1.58 / 1.82 | ≤ 1/4 |
| R4-7 | Typed finalized projection; custody in-process total checked | 1.05 / 1.16 | custody ≤ 0.51 s generic, ≤ 1.21 s multi-wallet (10× Python) |
| R4-8 | Typed fusion to `LedgerRequest`, `ledger_typed` in the engine, text only for refusal fidelity and comparison | builder 1.31 / 1.42, wire 0.19, engine parse and serialize about 0.08 | request ≤ 0.10 s; engine no slower |
| R4-9 | Native SQLite loader to shared custody rows, with strict typed JSON admission retained separately; accepted determinism continuation combines lossless BLOB parity and exact typed revision refusals, closing all forty historical seed-dependent message exceptions. Source acceptance is inherited from t3code/r4-9-loader-determinism@aaf6216e3. Fresh R9 loader/BLOB, generated/replay and focused gates pass; the R9 audit tracks combined verification. Earlier mutation and timing receipts retain their historical scope; Python retains production authority | Python load 1.57 / 1.89 (historical floor 0.11; remeasured shell CPU 0.117) | ≤ 0.30 s |

| R4-9 | Native SQLite loader to `BookSnapshot`, including book-network facts, the owned index and receipt sources; fully accepted with BLOB parity and deterministic revision refusals, no pending exceptions; all 357 loader and inherited 2,292 custody recipes killed; 24,000 generated books/1,030 permutations, 159 retained replays/permutations and focused census/shadow pass; fresh 22,528-case composed and 16,384-case non-BLOB audits match exactly; inherited 20k CPU including admission is 0.288 s generic / 0.429 s multi-wallet, within 5% of loader-7; Python retains production authority | Python load 1.57 / 1.89 (historical floor 0.11; remeasured shell CPU 0.117) | ≤ 0.30 s |
| R4-10 | Pre-build repairs and auto-pricing in Rust; database-dump parity. [Combined interface](#r4-10-combined-pre-build-contract) and later-failure rollback are implemented; campaign and CPU acceptance remain open. | Python 0.03 | ≤ 0.03 s |
| R4-11 | Rendering and joins: entries, quarantine enrichment with ownership candidates, holdings, balance comparison, ownership review; partial implementation, boundary continuation; acceptance remains open | Python 1.38 / 1.71 | ≤ 0.15 s |
| R4-12 | Persistence: typed rows to prepared statements, freshness markers, filed-report impact resolution, deferred checkpoint; database-dump parity. Unregistered checkpoint; acceptance open. | Continuation plaintext 20k fresh CPU: Python 1.504 / 1.553; floor 0.854 / 0.951; native 0.931 / 1.117. Unchanged CPU 0.770 / 0.453 still misses. | ≤ floor + 0.15 s |
| R4-13 | CLI `journals process` with and without `--auto-pair`, desktop `ui.journals.process` with pairing and scoped AI processing without pairing; progress phases; hybrid routing. Test-only composition checkpoint composes the real R4-10 API; refresh coordination and registration/acceptance gates remain open. | — | End-to-end targets below |
| R4-14 | Automatic pairing (transfer suggestions, rules, bulk exact) for `--auto-pair` and the desktop path; [stage accepted](rust-pairing.md), journal routing awaits R4-13/R4-15/R4-16 | Python 3.44 / 4.74 historical; CPU remeasured 4.710 / 5.823 | 10× Python CPU; native 0.362 / 0.495, **13.02× / 11.75×** |

| R4-9 | Native SQLite loader to `BookSnapshot`, including book-network facts, the owned index (descriptor derivation from the R5 descriptor slice) and receipt sources; v1 byte gate against Python; direct-load checkpoint: generic 20k 0.505 s CPU / 0.509 s wall including admission, target missed, acceptance remains open | Python load 1.57 / 1.89 (historical floor 0.11; remeasured shell CPU 0.117) | ≤ 0.30 s |
| R4-10 | Pre-build repairs and auto-pricing in Rust; [functional acceptance](#r4-10-functional-acceptance) completes database-dump and mutation gates. The scoped-cache multi-wallet CPU budget remains missed. | Python 0.03 | ≤ 0.03 s |

| R4-9 | Native SQLite loader to `BookSnapshot`, including book-network facts, the owned index and receipt sources; accepted loader-9 and BLOB continuations; 40 measured seed-dependent messages pending the Python fix; all 308 loader and 2,292 custody recipes killed; 24,000 generated books/1,030 permutations, 159 retained replays/permutations and focused census/shadow pass; inherited non-BLOB audit has 16,384 matches, zero writes/unsafe admissions; inherited 20k CPU including admission is 0.288 s generic / 0.429 s multi-wallet, within 5% of loader-7; Python retains production authority | Python load 1.57 / 1.89 (historical floor 0.11; remeasured shell CPU 0.117) | ≤ 0.30 s |
| R4-10 | Pre-build repairs and auto-pricing in Rust; database-dump parity | Python 0.03 | ≤ 0.03 s |
| R4-11 | Rendering and joins: entries, quarantine enrichment with ownership candidates, holdings, balance comparison, ownership review; partial implementation, acceptance remains open | Python 1.38 / 1.71 | ≤ 0.15 s |
| R4-12 | Persistence: typed rows to prepared statements, freshness markers, filed-report impact resolution, deferred checkpoint; database-dump parity. Unregistered checkpoint; acceptance open. | Python 2.25 / 2.55 (deferred-checkpoint floor 1.25 / 1.45) | ≤ floor + 0.15 s |
| R4-13 | CLI `journals process` and daemon `ui.journals.process` without automatic pairing in the binary; progress phases; hybrid routing | — | End-to-end targets below |
| R4-14 | Automatic pairing (transfer suggestions, rules, bulk exact) for `--auto-pair` and the desktop path (decision 3: ported in R4) | Python 3.44 / 4.74 | 10× Python |
| R4-15 | Freshness `journal_refresh` job and the daemon's single execution slot in Rust | — | Parity with the daemon smoke sessions |

| R4-15 | Freshness `journal_refresh` job and the daemon's single execution slot in Rust; integration checkpoint and refusal census | — | Parity with the daemon smoke sessions |
| R4-16 | Authority switch (below) | — | All gates and budgets |

The owner accepts borrowed read floor + 0.15 process CPU seconds for unchanged
R4-12 cells and prepared DML write floor + 0.15 for fresh cells. Measure the
read floor over five independent connections per shape and driver, use its
minimum, and retain process-CPU samples and machine load. The
persistence checkpoint
now retains the closest exact comparison under the owner's incremental
optimization rule. The full eight-cell budget gate and R4-12 acceptance remain
open; the withdrawn probes remain historical evidence.

R4-2 to R4-8 change no product behavior. Python still owns production until
R4-16. Order follows the data flow, because every stage reads the rows. A
stage whose neighbor is not typed yet would convert at its boundary. The
probes show that this can consume its gains. The R4-4/R4-5 kernel continuation
therefore retains both boundaries, introduces no such conversions, and
accepts optimizations by process CPU time across five alternating pairs.
See the checkpoint for its bounded touched-file mutation rerun and the
remaining whole-pipeline budget.

### R4-10 auto-pricing checkpoint

The checkpoints below retain their original gate status. The later
[functional acceptance](#r4-10-functional-acceptance) records the completed scope
and remaining CPU budget debt.

This partial, unregistered library checkpoint describes cached pricing. Later
checkpoints add the [source-overlap stage](#r4-10-source-overlap-checkpoint) and
[legacy refresh](#r4-10-legacy-refresh-checkpoint), then combine the ordered stage.
Python's ordered pre-build contract is:

1. `_repair_journal_source_overlaps`, followed by
   `_journal_source_overlap_warning`. Duplicate exclusions use audited
   metadata edits; unresolved or conflicting reviews prevent address-list
   trimming. Trimming updates wallet configuration, clears the matching UTXO
   inventory and resets its on-chain freshness checkpoint.
2. `refresh_drifted_legacy_pair_components`. Only active migration-authored
   `legacy_review_migration` components whose sole issue is an evidence
   commitment mismatch, with valid anchors, qualify. Connected pair groups
   migrate atomically; migration issues and component revisions remain part
   of the result. Re-resolve the profile when this stage changes it.
3. `auto_price_transactions_from_rates_cache`, then the snapshot/build.

The native pricing interface is
`kassiber_journals::prebuild::auto_price_transactions_from_rates_cache(
&Connection, &PricingScope) -> Result<usize, Refusal>`. `PricingScope` supplies
the refreshed profile id, its fiat currency and the explicit decimal context.
The Python journal runtime uses precision 32 and half-even rounding; pass
`Context::new(32, Rounding::HalfEven)`. The count includes only rows whose
four numeric/exact price columns were all SQL NULL before pricing; legacy
provenance enrichment and replacement of zero/negative prices do not count.

The caller must already own a write transaction (`BEGIN IMMEDIATE`) and the
whole-pipeline savepoint. Pricing refuses transactions without a writer
reservation, uses an inner savepoint, restores its writes on refusal or
unwinding, retains earlier caller writes and never commits. A later build or
persistence refusal must still roll back the caller's entire pipeline savepoint, including repairs
and successful pricing. The function does not validate tax policy, acquire a
book, repair sources or components, or refresh a profile on the caller's behalf.

Rate selection reads only `rates_cache`, including uncommitted caller rows.
It parses timestamp bounds at microsecond precision, preserves Python's
timestamp/source/fetched-at ordering, and prefers `confirmed_at` when present.
Stored exact transaction prices retain Python's precedence. The same legacy
and exact values and all provenance columns are written, including clearing
`pricing_external_ref`; profile freshness and metadata history are left to the
same Python triggers/caller contracts. A connection-local deterministic
`kassiber_timestamp_us` SQL function is registered only if pricing candidates
exist and the function is absent. No provider, client, consent token,
filesystem location, clock or entropy source enters the interface.

`tests/rust_port/journal_prebuild_oracle.py` records complete logical dumps,
including rowids, SQL storage classes, exact text, REAL bits, and refusal
causes. The checked-in representation shares identical tables and columns;
tests reconstruct the complete expected dump before comparing every table.
`scripts/check-journal-prebuild.py` repeats comparisons on real plaintext and
SQLCipher files. Its native subprocess runs under a Linux x86-64 syscall
filter that kills socket creation, connect and send calls, including loopback;
a socket-construction self-test verifies the filter. The Python oracle also
forbids DNS and socket connection calls.

The original pricing replay passed **106 comparisons** (53 pricing fixtures
on each driver), including a cache miss with market-rate refresh enabled. Its
pricing subset in `crates/journals/scripts/prebuild_mutations.json` had
**15 entries**, all killed by their declared named tests after the seven-test
baseline passed. These original results cover pricing only; source repairs
have separate evidence below.

Checkpoint checks pass: 18 journal library tests and six pricing integration
tests, the reproducible Python oracle, Rust formatting and clippy with warnings
denied, the required classifier update and all 57 Python preflight checks.

Python currently skips `L-BTC`, although the tax reference promises that alias;
only `LBTC` maps to BTC in the reference implementation. This checkpoint
preserves that behavior. Run
`uv run --locked python -m tests.rust_port.repro_journal_lbtc_price_alias`
for a disposable in-memory repro. Python is unchanged.

The pricing-only benchmark uses disposable 20,000-transaction books and five
Python/native pairs. CPU spans include the pricing query, cache lookup and
writes, including the native inner savepoint. They exclude book opening,
writer acquisition, fixture construction, dump verification and final rollback;
neither runtime commits the timed work. The three activities are an already
priced book, all prices missing with an empty cache, and all prices missing with
one stored rate. These are controlled pricing fixtures, not the complete
generator books or an end-to-end journal measurement.

| Driver / activity | Python median CPU (s) | Native median CPU (s) |
| --- | ---: | ---: |
| SQLite, priced | 0.009731 | 0.008097 |
| SQLite, missing cache | 2.100137 | 0.076979 |
| SQLite, cache fill | 3.160830 | 0.490466 |
| SQLCipher, priced | 0.021639 | 0.021772 |
| SQLCipher, missing cache | 2.368344 | 0.105330 |
| SQLCipher, cache fill | 3.609193 | 0.621775 |

The ≤0.03 s ceiling is met for the already-priced pricing stage on both drivers;
both missing-price activities exceed it. The repair stages are unmeasured.
Raw CPU samples and driver replay evidence
retain all five measurements. Reproduce with a release `journal-prebuild`
example (`--features test-harness`) and
`uv run --locked python scripts/check-journal-prebuild.py --binary
<absolute-example-path> --benchmark --out <target/results.json>`.

Remaining R4-10 acceptance work: compare the twelve generators with
per-scenario activity, recorded books and
fresh recordings on both drivers; expand the legacy/refusal corpus beyond
these bounded decimal and timestamp fixtures; complete the inherited census,
shadow, frozen-vector and whole-catalog mutation gates. The pricing-only
fixtures and bounded mutation catalog do not establish those broader gates or
the complete pre-build CPU target. No production route calls this checkpoint.
The later [scenario replay checkpoint](#r4-10-scenario-replay-checkpoint)
adds bounded whole-stage coverage without closing those acceptance gates.

### R4-10 source-overlap checkpoint

The unregistered native source stage exports
`prebuild::repair_journal_source_overlaps(&Connection, profile_id, clock, identity)`
and `prebuild::source_overlap_warning(&Connection, profile_id, repair.as_ref())`.
Call them in that order, before legacy refresh and cached pricing. The repair
returns `Result<Option<serde_json::Value>, Refusal>`; the warning has the same
return type. Their optional values are exactly the Python
`source_overlap_repair` and `source_overlap_warning` result projections. The
`prebuild::source_overlap_result_fields(repair.as_ref(), warning.as_ref())`
helper returns a lossless `kassiber_pyjson::Value` dictionary to append after
the ordinary journal result fields. It preserves Python's nested dictionary
insertion order for byte-identical machine responses; serializing the
`serde_json::Value` results directly does not preserve that order. The
caller supplies fallible clock and identity callbacks. For each changed
duplicate, the repair samples the clock once, then requests one event ID
followed by one field ID. Temporary savepoint identities are not part of the
callback contract.

The repair requires an existing writer reservation, owns an inner savepoint,
and never commits. Refusals and unwinding restore every repair write while
retaining pending caller writes. A successful return still belongs to the
caller's whole-pipeline savepoint. The warning performs local reads only.
Neither operation opens a connection, reads a filesystem, or contacts a
configured backend. Descriptor derivation reuses the existing local wallet
implementation and its finite 20,000-target cap per branch.

The source index preserves address-list/inventory/descriptor evidence,
network separation, deprecated wallets with active rows, and Python's
canonical recommendation and ordering. Duplicate matching requires raw script
evidence and the complete external-ID/direction/asset/amount/fee fingerprint.
Exclusions append metadata history and invalidate the profile exactly as
Python does; active custody reviews produce skipped records. Outstanding
duplicates, conflicts, and unmatched active address-list rows prevent trimming.
Trimming preserves unrelated inventory and resets only that wallet's on-chain
freshness checkpoint.

`tests.rust_port.journal_source_overlap_oracle` freezes complete before/after
dumps and result/refusal projections for 49 synthetic scenarios, including
serialized result-field bytes. Its ID tape includes only durable audit
identities; the clock tape includes one timestamp per audited exclusion and
has a changing-clock scenario. Native tests also inject a late SQL refusal,
a clock refusal, and an identity refusal/panic after earlier exclusions.
Legacy numeric and integer-text fingerprints retain Python's conversion and
refusal behavior; nonempty invalid amount/fee text refuses instead of becoming zero. An
absent deprecation flag remains distinct from invalid deprecation text.
Native tests also reject positive/negative SQLite infinity in amounts and
fees before serialization can erase it into a JSON null, preserving every
stored scalar and avoiding any audited exclusion on that refusal.
Production routing and the Python reference remain unchanged.

The final source/pricing replay passes **204 comparisons**: 49 source and
53 pricing scenarios on each real SQLite/SQLCipher driver, under the native
socket syscall guard. The bounded catalog has **46 named mutations** killed
across a full 45-entry campaign and the subsequent focused infinity-guard
campaign. The full campaign preceded the final scalar guard and narrower
audit SELECTs; after those changes the final 33-test native baseline, all 46
recipe applicability checks and the 204 driver comparisons pass. Formatting,
clippy with warnings denied, classifier update (no inventory delta), and all
57 Python preflight checks also pass. Source checkpoint evidence and named
mutation mapping retain these boundaries;
they do not establish the inherited whole-stage campaign or CPU target.

The Python reference also truncates fractional legacy msat when constructing
source fingerprints: distinct SQLite REAL amounts 1.25 and 1.75 both become
1 and can cause an audited exclusion. Run
`uv run --locked python -m tests.rust_port.repro_journal_fractional_source_overlap`
for a disposable in-memory repro. This port retains the reference conversion;
rejecting those invalid legacy quantities needs a separate Python correction
and refreshed parity vectors.

Legacy refresh retains the distinction between migration domain refusals
(record a skipped migration issue) and SQLite failures (propagate and roll
back). The private component-authoring helpers in `crates/transfers` normally
map SQLite errors to `AppError`; the refresh preserves their raw errors in a
bounded scoped capture so the journal boundary can propagate their exception
family. The combined interface below composes the stages for R4-13; production
routing remains outside this checkpoint.

### R4-10 legacy-refresh checkpoint

`prebuild::refresh_drifted_legacy_pair_components(&Connection, &serde_json::Value,
clock) -> Result<LegacyRefresh, Refusal>` accepts the resolved profile and a
fallible `FnMut() -> Result<String, Refusal>`. `LegacyRefresh` contains
`consolidation` (`activated`, `unchanged`, `skipped`) and `profile`. After an
activated **or skipped** outcome, `profile` is reloaded from the same connection;
otherwise the supplied projection is retained. Later pricing, loading and
persistence must use this returned profile. The migration count is per
connected group, rather than per legacy row.

Only active migration-authored legacy pair components with valid anchors and
exactly the evidence commitment mismatch issue qualify. Existing economics,
unrelated authored components, deleted rows and unlinked legacy rows do not
gain authority. Every linked member of a connected pair group must qualify;
unlinked neighbors remain part of the same atomic migration. Revisions,
canonical economic row hashes, source residuals, evidence snapshots,
commitments, memberships, legacy links and migration issues use the shared
transfer authoring implementation. The journals crate adds only an internal
dependency on that implementation, without an external package.

The caller must own a reserved writer transaction. An outer stage savepoint
and inner group savepoints retain caller writes and restore all stage writes
on technical refusal or unwinding. A domain refusal restores the partial group
before recording its durable issue. SQLite failures retain their raw exception
family and propagate. Clock failures preserve the supplied refusal; clocks are
sampled only where Python samples them. No network, filesystem, implicit clock
or entropy source enters the interface.

`tests.rust_port.journal_legacy_refresh_oracle` freezes **40 complete logical
dumps** and result/refusal projections. They include connected split and
conversion groups, mixed linked/unlinked groups, two independent groups,
changing clocks, legacy REAL/integer text, malformed links, author/evidence
eligibility, prior issues, explicit timestamp normalization and domain refusals
after partial activation, plus successful and refused second-drift revisions.
Native tests also inject clock refusal/unwinding and
a late SQL failure after an earlier group's successful work. Both changed
outcomes independently prove profile reload. The bounded legacy mutation
catalog is `crates/journals/scripts/legacy_mutations.json`; the inherited
whole-stage campaign and CPU gates remain open. No production route calls this
checkpoint.

The original R4-10 separate-stage replay passed **280 full-dump comparisons**: all 53 pricing,
49 source and 38 legacy cases on each real driver under the native socket
guard. All seven legacy tests and 41 transfer tests pass, as do the existing
journal unit, render, join, persistence, source and pricing suites. The
checkpoint evidence records the exact scope;
these results do not establish whole-stage campaign or performance acceptance.
The R4-13 continuation expands legacy coverage
to 40 books and eight native tests. Its real-driver replay passes **300
comparisons**, including the combined interface's eight cases on both codecs.

### R4-10 combined pre-build contract

`prebuild::prepare_journals(conn, profile, decimal_context, clock, identity)`
returns `Result<Prepared, Refusal>`. It runs source repair, source warning,
legacy refresh/profile reload, then cached pricing in that order. `Prepared`
contains `repaired: RepairedInputs` and `auto_priced: usize`. `RepairedInputs`
contains the returned `profile`, optional `source_overlap_repair` and
`source_overlap_warning`, and the legacy `consolidation` counts. Use the returned
profile for every later loader, builder and persistence call. The explicit
decimal context is precision 32, half-even, as in the pricing interface.
Clock and identity callbacks retain the individual stages' ordering and
refusal semantics; only source audit records request identities. No stage
contacts a provider, including on a cache miss with auto-refresh enabled.

For R4-13's separate repair and price progress events, call
`prebuild::repair_journal_inputs(conn, profile, clock, identity)` to obtain
`RepairedInputs`, then `auto_price_transactions_from_rates_cache` using its
returned profile. This split retains the same ordering as `prepare_journals`.
Map `source_overlap_repair`, `source_overlap_warning`, and
`consolidation.activated` to R4-13's `Repairs`; the last is
`custody_pair_evidence_repairs` and is emitted only when nonzero. Append
`RepairedInputs::result_fields()` after ordinary journal fields to preserve
Python's nested dictionary insertion order. `auto_priced` remains an ordinary
journal field. Never serialize the optional `serde_json::Value` projections
directly when byte parity is required.

All three entry points require the caller's existing writer reservation
(`BEGIN IMMEDIATE`); none commits. The repair and combined interfaces own
nested savepoints. The caller must keep its whole-process savepoint alive
through snapshot loading, build and persistence, so a later refusal restores
successful repair/pricing writes as well. Alternatively,
`prebuild::with_prebuild(conn, profile, context, clock, identity, continuation)`
owns this enclosing savepoint and invokes a fallible continuation with
`&Prepared`. It returns `(Prepared, continuation_result)` on success. Any
refusal or unwind restores every database write since entry while retaining
earlier caller writes and leaving the writer transaction open. Final commit
and rollback remain with the caller. Callback consumption and side effects
outside the connection cannot be rewound.

The combined Python oracle freezes eight complete before/after dumps,
including a book with all three stages active, a skipped migration, warning,
repeat, and refusals after earlier successful repair work. Native rollback
tests execute an actual journal build refusal and inject a failure or panic
after every write in actual acquisition persistence, comparing every table
after restoration. This scope establishes the composition contract, not the
whole-stage campaign or performance target.

At the original combined-interface handoff, all **61 journal tests** passed,
including the six combined-interface tests.
The Python combined oracle reproduces its eight frozen vectors, and the real
SQLite/SQLCipher replay passes **16 combined-stage full-dump comparisons**
under the socket syscall guard. These extend the earlier 280 separate-stage
comparisons; that unchanged separate-stage corpus was not rerun to completion
at this boundary. Release clippy with warnings denied and all seven pipeline
mutation recipe applicability checks pass. Mutation execution belongs to the
coverage campaign. The required classifier refresh leaves no inventory delta,
and all **57 Python preflight checks** pass before this checkpoint commit.
Checkpoint evidence
records this scope.

### R4-10 scenario replay checkpoint

The whole-stage corpus now includes **505 stored database scenarios** drawn
from all twelve custody generators, with one frozen operand per representable
scenario. Python regenerates every before/after table, rowid, scalar, result,
refusal and serialized optional result field. The native test replays the same
operands, checks callback consumption, and retains the existing caller rollback
checks. These are scenario representatives, not the 24,000-book campaign.

`tests.rust_port.journal_prebuild_campaign` explicitly adapts snapshot columns
to database operands. It omits snapshot-only joins, ownership indexes and receipt
projections; it creates no inventory or legacy pair links from them. It adds
synthetic stored cache rates and required scope/identity metadata, and records
omitted columns plus label/fingerprint adjustments. Legacy malformed CHECK
values can be loaded with checks temporarily disabled during fixture creation;
checks are enabled again before pre-build. Ten scenarios cannot be stored
because of uniqueness or required-field constraints. Their named schema refusals
are frozen separately and do not count as native pre-build comparisons. This
adapter does not prove an exact custody snapshot round trip or new source/legacy
activity; the activity matrix reports those distinctions.

All **1,010 complete-dump file comparisons** pass: these 505 operands on both
plaintext SQLite and SQLCipher files, under the native socket syscall guard.
The probe reuses a connection per driver to avoid repeated key derivation,
recreates each fixture, restores FULL synchronization before the stage, and
rolls back the tested stage. Fixture setup and cleanup have no durability claim.
The existing separate-stage fresh-open evidence remains separate.

`scripts/check-journal-prebuild-campaign.py` supports the exhaustive generated
campaign, frozen operands and complete full-book recordings. `--files` selects
both real file drivers; without it comparisons use the native driver in memory.
Completed summaries are explicitly marked complete; an interrupted run cannot
supply acceptance evidence. The all-file and all-seed generated campaigns were
interrupted at this checkpoint and are **not passed gates**.

The opt-in pytest plugin `tests.rust_port.journal_prebuild_recorder` captures the
whole ordered stage before the original source repair runs. It copies the actual
schema and every stored row, including pending caller writes, to the guarded
native probe. Its oracle uses a rollback-only savepoint, leaves the caller's
transaction open, and never commits pending work. Recordings require an explicit
disposable directory below this checkout's `target/`; each file is private.
Comparison failures also fail the pytest session if a tested handler catches the
assertion. This is local verification tooling with no product or AI route.

The coverage record gives the measured
scope and per-scenario activity. At this checkpoint R4-10 was unaccepted:
exhaustive generated and historical recorded replay, broader legacy/refusal inputs, the complete
branch mutation campaign, whole-stage 20k CPU profiling/cache optimization, and
the final acceptance record remained open. The Python `L-BTC` omission and
fractional-msat fingerprint truncation remain unchanged.

The checkpoint passes all **62 journal Rust tests**, two Python corpus/transaction
tests, release all-target Clippy with warnings denied, and Rust formatting.
The final inventory update records the two Python oracle tests, and the required
CI preflight passes **57 tests**.

### R4-10 generated and recorded campaign

The exhaustive stored-operand campaign now completes all **24,000 generator
seeds and 1,030 permutations**. Its private activity log binds every operand
to an input digest and the tested binary. Resume validates the retained input
prefix and driver/binary identity before reusing comparisons; changed inputs
are refused. Activity is flushed before summary checkpoints. The final summary
was rebuilt from the complete retained log; its elapsed time measures this
validation, not the original comparison campaign.

| Corpus | Stored operands compared | Complete SQLite/SQLCipher file comparisons |
| --- | ---: | ---: |
| Twelve generators and permutations | 24,815 | 49,630 |
| Historical custody recordings and permutations | 318 | 636 |
| Three historical upgraded databases, one profile each | 3 | 6 |
| Fresh complete books and positive control | 89 | 178 |

The **215 schema refusals** among generated attempts remain separate from
native pre-build parity. They cover ten unrepresentable scenarios, with no
native operand created. Fifteen representable operands produce matching
pre-build refusals. Generated operands auto-price 25,887 rows, while 280
storable scenarios exercise no repair, pricing, warning or refusal activity.
Per-scenario counts, omitted columns and distinct input counts are retained in
the campaign record. Repeated generator
inputs are disclosed; 24,000 attempts does not mean 24,000 distinct books.
The historical snapshot replay uses the same limited adapter. Full historical
database replay starts after Python upgrades disposable copies of the three
exact fixtures (`pre_432_5d232097`, `pre_435_16b7bdc1` and
`pre_partial_payout_ca9a672f`), with one profile in each; it does not extend
native migration support.

Fresh recordings preserve actual stored source evidence. Across 89 captures
they exercise 27 duplicate exclusions, three address removals, one warning and
14 priced rows. They produce no legacy activation or skip; broader legacy
coverage remains a separate gate. The nine-module fresh run passes 398 tests
and 20 subtests, with five sandbox failures. The five failing tests pass in a
focused rerun with local permissions, an initialization prerequisite and a
positive pre-build control: seven passing tests and one comparison. Four
failures were denied loopback test sockets; the chat consent-refusal test also
passes with the needed local permissions. No reference or product code was
changed for these controls.

The recorder now propagates explicitly to Python children, gives each capture
a process/sequence identity, and writes private failure markers that the parent
session checks even when a child catches the comparison assertion. Sessions
require empty recording directories. Automated controls prove rejection of a
swallowed child mismatch, stale directories, zero-comparison sessions and a
changed resume prefix. The six-test permission rerun without a pre-build caller
also correctly exits unsuccessfully despite passing test bodies; adding the
positive control makes the session green.

R4-10 remains **unaccepted**. The legacy/refusal and oversized-number expansion,
remaining census/shadow lanes, mutation execution and branch audit, cache
optimization and final CPU/acceptance record are outstanding. Initial five-pair
whole-stage CPU medians are 0.057464 s generic 20k and 0.072883 s multi-wallet
20k, above 0.03 s. These initial native probes use the default page cache;
the performance continuation must measure the existing scoped rebuild cache
and report the complete stage cost. The campaign record retains all raw CPU
samples and benchmark-book identities.

### R4-10 legacy and refusal expansion

The separate-stage corpus now has 57 legacy, 69 pricing and 53 source cases.
The ordered corpus has 50 cases, including the expanded edge operands and
refusals after successful source repair, legacy activation, and an earlier
independent legacy group. All before/after dumps preserve rowids, storage
classes, exact decimal text, REAL bits, callback consumption, and result-field
bytes. The native tests also retain pending caller writes and prove rollback.

Reviewed amount text covers Python Unicode/underscore syntax, malformed,
fractional and exponent text, positive and negative i128 boundaries, 4,300-digit
accepted inputs, the 4,301-digit refusal, and leading-zero digit-limit counting.
Missing anchors/commitments and a recurring resolved migration issue remain
separate eligibility or refusal cases. Legacy integer conversion now preserves
its raw Python exception family through the transfer helper; it does not become
a skipped domain issue. Oversized positive reviewed quantities retain their
exact value in the domain issue before any fixed-width conversion; nonpositive
quantities cannot activate a slice. The scoped raw SQLite capture remains
unwind-safe and retains technical exception precedence.

Pricing covers decimal construction limits, arithmetic overflow/subnormal
results, 5,000-digit coefficients and NaN payloads, legacy REAL/text quantities,
missing caches, unsupported assets, existing values and value-derived rates.
Quantity conversion happens only when Python needs it, after cache selection.
The precise chained decimal refusal survives construction overflow.
Python also truncates a fractional legacy quantity during pricing; reproduce
with `uv run --locked python -m tests.rust_port.repro_journal_fractional_pricing`.
The existing alias and fingerprint bugs and this quantity truncation remain
unchanged in Python and are retained for parity.

All **458 complete-dump comparisons** pass across real SQLite and SQLCipher
files under the native socket guard; its self-test passes. Release all-target
Clippy passes for both affected crates with warnings denied. Seven additional
mutation recipes pin the corrected refusal paths; execution remains open.

These are additional correctness checkpoints. Census/shadow, full mutation
execution, cache optimization and whole-stage CPU acceptance remain open.

### R4-10 mutation and census checkpoint

All 81 pre-build recipes are now killed: 21 legacy, seven pipeline, 46
pricing/source and seven numeric-edge recipes. The initial run killed 80;
`JL-retarget` survived because a later retarget compensated for the deleted
operation. A new Python-authored fixture keeps an inactive deterministic
revision beside a different active migration revision. That fixture kills the
survivor and is frozen in both the 58-case legacy and 51-case ordered corpora.
The previous frozen cases retain their operands and expectations.
All **462 complete-dump comparisons** pass across SQLite and SQLCipher under
the native socket guard. The release journal/transfer suite passes 107 tests.

Fresh handler-path controls now record both a legacy activation and a domain
skip in each focused census/shadow lane. Both outcomes pass complete replay
on SQLite and SQLCipher. Earlier low-level processing tests supplied their own
source callbacks, so the recorder did not observe their legacy work.

Broader runs exposed two recorder installation defects: oracle-first child
imports could encounter a partially initialized oracle, and an import hook
could install a second wrapper reentrantly. Oracle/checker imports are now
lazy, installation checks again after importing handlers, and each wrapper
retains its original callback. Regressions cover all three child import orders
and force re-entry while requiring exactly one capture. Later report runs found
that the first `resource` import inside the pre-exec syscall guard emitted
`-X importtime` diagnostics into the native probe's captured stderr. The checker
now imports it in the Linux parent. Platform refusal, the core-dump limit,
socket filter and strict stderr assertion remain intact. A positive import-time
regression passes, and all three Python recorder/guard mutations are killed
after their unmutated controls pass. Swallowed child differences, stale
directories and zero-comparison sessions still fail.

The custody harness's own unit tests now isolate their mode lifecycle from an
outer census or shadow session and restore that session afterward. This keeps
their deliberately patched refusal cases compared instead of accidentally
treating them as patches made after installation. Product/reference accounting
code and comparison rules are unchanged. Both focused harness lanes pass
89 tests, with 19 pre-build captures and four exact native-loader matches each.
Recordings include the originating pytest node; regeneration of the 505 frozen
generator operands is not counted as that many fresh handler books.

The full core census initially passed 1,916 tests with 167 failures in the sandbox.
A permitted continuation passed 1,883 tests before stopping at the recorder
import defect; focused reruns close every originally failed case, including
nine remaining environment cases. These are combined-run results, not a claim
that one uninterrupted shard passed. The initial full run recorded 333 exact
native-loader matches and 568 matching pre-build captures. Books remain
disposable under `target/`, with the account `HOME` unchanged.

The full core custody-shadow rerun passes **728 tests and 234 subtests**, with
328 exact loader matches. The report census and shadow runs pass 2,300 and
1,822 tests before the import-time guard defect stops both at two subtest
failures. After the guard correction, exact JUnit-selected continuations pass
**231 census tests and 103 subtests**, and **109 shadow tests and 42 subtests**.
Collection manifests prove coverage of all 2,540 census and 1,940 effective
shadow identities, with no missing identities or unrechecked failures. Shadow's
two deliberate single-build SQL-read-budget exclusions pass in census.
These report results are combined-run coverage, not uninterrupted passing shards.

| Lane | Pre-build captures | SQLite/SQLCipher file comparisons |
| --- | ---: | ---: |
| Full core custody-shadow | 568 | 1,136 |
| Report census, initial and completion | 167 | 334 |
| Report custody-shadow, initial and completion | 167 | 334 |

The core captures include 505 regenerated frozen operands (793 priced rows and
one refusal) and 63 other handler captures (five priced rows). Each report lane
captures one legacy activation, one skip and seven warnings. The two focused
19-capture harness sets also pass all 76 file comparisons. Counts describe
captures, not distinct books; originating test nodes and activity remain in the
record. Failed initial sessions are not presented as standalone passing gates.

R4-10 remains **unaccepted**. The inherited journal (112), transfer/daemon (280)
and custody (2,327) mutation catalogues apply, but their execution, the branch audit,
whole-stage cache/CPU work and final acceptance record remain open. The
campaign record retains the exact scope.

### R4-10 uncovered numeric paths

Instrumented replay found a refusal mismatch for binary numeric storage. Python
passes the bytes representation through its decimal parser and raises a chained
`AppError`; native pricing had raised an unrelated `TypeError`. Native pricing
now retains Python's quote choice, escaping and decimal cause. Empty bytes are
not treated as missing prices, and an earlier successful price write still rolls
back with the later refusal. The Python reference is unchanged.

The corpus adds 17 pricing and 17 ordered cases: binary cache/transaction prices,
all 256 byte values, lazy precedence, positive/negative infinity, signaling NaN
without quantity, and infinite legacy quantities. All previous frozen operands,
expectations and shared bases are unchanged. The corpus now has 86 pricing and
68 ordered cases, and all **530 SQLite/SQLCipher file comparisons** pass under
the socket guard.

Those cases also exposed an incomplete comparison format: strict Python JSON
could not encode infinite stored REALs, while the native dump encoded them as
null. Private dumps now use `$real_bits` with sixteen hex digits encoding binary64 bits for
nonfinite REALs. Both fixture loaders restore the storage class and sign; all
finite scalar representations remain unchanged. This changes no book schema,
stored production format or product protocol. SQLite round-trip controls and
the complete stage comparisons prevent null substitution from passing.

All **109 release journal/transfer tests**, **10 Python harness tests**, and
**19 new numeric/dump mutations** pass; every mutation is killed by its named
test after its baseline passes. The three recorder/guard mutations are also
killed again. Formatting and release all-target Clippy pass with warnings denied.

The refreshed coverage run passes 35 pre-build tests. Pricing executable-line
coverage increases from 294/329 to 328/346, and the active bytes formatter has
27 calls with no zero-count regions. The isolated coverage build tree is removed
after export. Stable LLVM instrumentation emits no branch counters: the 209
remaining zero-count region entries include generic instances and defensive paths,
so they are not 209 distinct untested branches. Their witness/disposition audit
is still open. The branch record binds source
hashes, raw exports, mutations and test artifacts.

R4-10 remains **unaccepted**. The remaining branch audit, inherited mutation
catalogues, whole-stage 20k cache/CPU work, final campaign binding and acceptance
record remain open. Earlier generated/census/shadow evidence retains its original
implementation binding; this checkpoint does not claim a fresh whole-campaign run.

### R4-10 inherited journal mutation gate

The inherited render/persistence catalogue now passes all **112 recipes**.
All 20 named baseline selections pass unmutated; every recipe builds and is
killed by its named tests. There are no survivors or build/selection errors.
The inherited gate record binds the
catalogue and mutated source hashes to the tested implementation and local log.
The shared checker uses a disposable copied tree, offline Cargo and one test
process at a time. Product code and the Python reference are unchanged.
The local summary writer failed after the successful checker returned; the
record explicitly distinguishes that bookkeeping error and its reconstruction
from the completed mutation gate.

R4-10 remains **unaccepted**. The 280 transfer/daemon and 2,327 custody recipes,
remaining uncovered-region dispositions, whole-stage cache/CPU work, final
campaign binding and acceptance record remain open.

### R4-10 input witnesses and inherited transfer gate

All **280 inherited transfer/daemon recipes** are killed: 276 Rust and four
Python mutations, after all 44 named baseline selections pass. There are no
survivors, build errors or selection errors. The inherited gate record
binds the catalogues, source hashes and complete local logs. The 112 journal
recipes remain verified separately; these are two completed gate runs.

The frozen corpus adds 22 source and ten pricing inputs, with all 32 also
captured through the ordered stage. They cover numeric iterable refusals,
comma-separated and blank addresses, canonical-row edge cases, scope and wallet
fallbacks, checkpoint merging, configured xpubs, special decimal division and
leap-year refusal precedence. Every previous case and shared base is unchanged.
There are now 75 source, 96 pricing, 58 legacy and 100 ordered cases; all
**658 complete SQLite/SQLCipher file comparisons** pass under the socket guard.
Native interface guards also reject missing scope/profile fields and absent
writer reservations before callbacks or writes, retaining pending caller writes.

All **114 release journal/transfer tests** pass. All **12 added mutations** are
killed after four named baseline selections pass. Release all-target Clippy
and formatting pass. The branch record retains
the exact vectors, mutation mappings and coverage export. Its 40-test coverage
run raises source executable-line coverage to 349/358, overlap to 490/495 and
pricing to 336/346. Stable instrumentation still emits no branch counters;
the remaining 122 zero-count executable-region entries require explicit
dispositions and are not a count of distinct untested branches. The isolated
coverage build tree is removed after export.

R4-10 remains **unaccepted**. The 2,327 custody recipes, remaining region audit,
whole-stage cache/CPU work, final campaign binding and acceptance record stay open.
The Python reference and product routing are unchanged.

### R4-10 branch and cache checkpoint

The final input expansion retains every previous frozen operand and expectation.
It adds linked legacy-review identity, unknown-network fallback, JSON-null
inventory and value-derived overflow witnesses. There are now **78 source,
97 pricing, 58 legacy and 104 ordered cases**. All **674 complete-dump file
comparisons** pass on SQLite and SQLCipher under the native socket guard.
Native guards also cover the second audit identity, missing tables, invalid
source storage and a scoped profile moved by an injected clock callback.

The refusal audit found that source/pricing trigger failures returned native
`DatabaseError` instead of Python's `IntegrityError`. The shared journal SQLite
conversion now preserves the DB-API family and original SQLite message; legacy
refresh uses the same conversion. Constraint and operational failures propagate
and restore the stage instead of becoming domain skips. Two Python/native
trigger probes match complete dumps and exact refusals. The Python reference,
book schema and production routing are unchanged.

The combined repair/warning path now shares the no-overlap result of its source
index. This shortcut is confined to one reserved writer call: that repair
cannot write or consume callbacks, and warning would observe the same index.
Standalone calls still read fresh evidence. Active repairs retain the existing
post-write index refreshes. Nothing is cached across calls, books or connections.

All **116 pre-build recipes**, **112 inherited journal recipes** and **three
recorder recipes** are killed in a fresh isolated build after all **57 named
baseline selections** pass. Catalogue-qualified names disambiguate overlapping
recipe ids in the local combined run. A preceding reused-debug-tree attempt
failed two unmutated baselines; its results are excluded. The clean debug
control and isolated gate pass. The mutation build tree is removed.

Fresh LLVM instrumentation passes **44 pre-build tests**. It leaves **104
zero-count executable-region entries**, each with a disposition in the
region audit. They include SQL/error propagation,
closed-input invariants, defensive descriptor handling and shared allocation
bounds. Stable LLVM emits no branch counters; this is not a claim of measured
100% branch coverage. Allocation-failure behavior above the inherited Decimal
formatting bound is outside the parity claim. The coverage build tree is removed.

The whole ordered stage is measured on the existing disposable 20k benchmark
books, with seven alternating Python/native pairs per cache setting. User plus
system process CPU includes callbacks, optional result fields, and scoped cache
entry/restoration; opening, fixture construction, writer acquisition, dump
verification and final rollback are outside the timer. Each sample still checks
its complete expected dump and result. Shared-machine load is retained in the
raw record; these are process-CPU measurements, not quiet-machine wall gates.

| 20k book | Before source-index reuse, default cache | Final default cache | Final scoped rebuild cache | Target |
| --- | ---: | ---: | ---: | ---: |
| Generic | 0.031240 s | 0.021221 s | 0.029070 s | 0.030000 s |
| Multi-wallet | 0.040664 s | 0.026913 s | 0.034666 s | 0.030000 s |

The scoped-cache multi-wallet median misses by **0.004666 s (15.6%)**. Its
best sample is 0.033108 s, so selection of the fastest sample does not close the
budget. The median diagnostic sample costs 0.023649 s in repairs, 0.009276 s
in pricing, 0.002645 s entering the cache, 0.000366 s restoring it and 0.000042 s
in other stage work (0.035978 s total). The larger cache adds entry cost and
raises repair-scan CPU while reducing pricing reads. The 128 MiB setting also
serves loading and persistence, so changing it for this read-heavy stage needs
whole-pipeline measurement in R4-13. The default-cache books meet the target;
the scoped-cache miss and complete costs remain recorded budget debt.

All **118 release journal/transfer tests**, **10 Python harness tests**, release
all-target Clippy, formatting, classifier refresh and **57 preflight checks**
pass. The finalization record binds
source hashes, vectors, logs, all CPU samples and coverage dispositions.
R4-10 remains unaccepted pending the custody mutation continuation, final
whole-campaign implementation binding and acceptance record.

### R4-10 inherited custody mutation gate

All **2,327 custody recipes** are killed, with no survivors, build errors or
selection errors. The inherited gate record
checks every catalogue id exactly once across verified **396 + 156 + 1,775**
recipe segments. The first two runs were intentionally interrupted; their
incomplete pending recipes were rerun. This is combined evidence, not one
uninterrupted catalogue run.

All 424 original baseline selections passed. The continuations separately
passed 138 and 128 selections. The last continuation built and listed all
613 tests across 11 custody test targets before its baseline, then built every
target containing a named witness for each mutation. Every named substring
filter selected tests and failed as required. Benchmark examples were excluded.
The record binds the runner, checker, catalogue, source files and complete logs.
Copied mutation trees and the isolated build tree are removed after validation.

The 280 transfer/daemon recipes retain their verified mutated-source hashes.
The 112 journal recipes were refreshed in the final-source 231-entry isolated
run after the shared SQLite refusal conversion changed. All inherited mutation
catalogues are now complete. Final whole-campaign binary binding and the
acceptance record remain open.

### R4-10 functional acceptance

The unregistered native pre-build boundary is **functionally accepted**, with
an explicit scoped-cache multi-wallet CPU-budget miss. The
acceptance record binds the final binary, sources,
activity logs, complete file comparisons, mutations and required checks.
R4-13 owns production integration and the remaining whole-pipeline cache tradeoff.

The final binary repeats all 24,000 generated seeds and 1,030 permutations.
Its 215 schema refusals remain separate from the 24,815 stored operands and
49,630 native file comparisons. Per-suite activity, omitted columns, refusals,
distinct request hashes and operands with no observable stage activity are
recorded. Repeated requests and regenerated frozen captures are counted as
operands, without claiming that they are distinct economic books.

| Final-binary corpus | Complete SQLite/SQLCipher file comparisons |
| --- | ---: |
| Twelve generators and permutations | 49,630 |
| 159 historical snapshots and their permutations | 636 |
| Three Python-upgraded historical fixtures, one profile each | 6 |
| 505 frozen representatives | 1,010 |
| Fresh recordings and network/positive control | 178 |
| Core census initial, permitted and completion captures | 2,270 |
| Core custody-shadow captures | 1,136 |
| Report census initial and completion captures | 334 |
| Report shadow initial and completion captures | 334 |
| Earlier focused legacy census/shadow captures | 8 |
| Focused census/shadow harness captures | 76 |
| Expanded hand-authored edge corpus | 674 |
| Fresh final handler census/shadow controls | 8 |
| **Total** | **56,300** |

The historical inputs remain byte-identical to their before-replay manifest.
The snapshot adapter grants no inventory, ownership or legacy-review authority.
The three database fixtures are upgraded in disposable copies by Python before
pre-build comparison; this does not extend native historical authored migration.

Census and report-shadow identity coverage remains the documented combination
of initial runs and exact continuations. Replaying a capture from a failed test
session proves that individual input's parity; it does not make that session
an uninterrupted passing shard. The final two-case handler runs pass in both
census and shadow modes, each observing one legacy activation and one domain
skip. The core frozen-regeneration captures remain separate from independent
handler activity in the recorded provenance.

All **2,838 catalogue entries** are killed: 116 pre-build, three recorder,
112 inherited journal, 280 transfer/daemon and 2,327 custody entries. The final
journal gate was refreshed after the shared SQLite refusal fix. Transfer gate
mutated-file hashes remain unchanged. Custody combines its verified
396 + 156 + 1,775 segments, with every incomplete pending recipe rerun.
There are no survivors or build/selection errors within the recorded build scope.

The default-cache 20k medians meet 0.03 s at **0.021221 s generic** and
**0.026913 s multi-wallet**. Scoped-cache medians are **0.029070 s** and
**0.034666 s** respectively. The latter misses by **0.004666 s (15.6%)**;
its best sample is 0.033108 s. The median diagnostic sample totals 0.035978 s:
repairs 0.023649 s, pricing 0.009276 s, cache entry 0.002645 s, restoration
0.000366 s and other stage work 0.000042 s. These are whole-stage user plus
system process-CPU measurements, including cache entry/restoration and result
fields. The unchanged shared 128 MiB cache serves later stages too; its tradeoff
requires whole-pipeline measurement in R4-13. The complete performance target
is still unmet for scoped-cache multi-wallet.

All 118 release journal/transfer tests, ten Python harness tests, release
all-target Clippy, formatting, the final classifier update and 57 preflight
checks pass. The region audit dispositions all 104 remaining zero-count
executable-region entries after 44 pre-build coverage tests. Stable LLVM emits
no branch counters, so no measured 100% branch-coverage claim is made.
Allocator exhaustion and formatting above the inherited Decimal allocation bound
remain outside the parity claim.

Python's `L-BTC` omission and fractional-msat truncation are retained, with
checked-in repros. Production pre-build routing and book schema are unchanged.
The summarized mutation and coverage build trees are removed, historical inputs
remain read-only, and verification leaves no owned background jobs running.

## Performance budget

The budget is for the Rust binary's `journals process` (no `--auto-pair`) on
plaintext books, from process start to exit, best of three at 2,000 and best
of two at 20,000. Lines marked *floor* are measured SQLite or Rust costs from
above. The other lines are targets.

| Line, seconds | generic 2k | multi-wallet 2k | generic 20k | multi-wallet 20k |
| --- | ---: | ---: | ---: | ---: |
| Start, open, scope (floor: R5 open) | 0.10 | 0.10 | 0.15 | 0.16 |
| Snapshot load | 0.03 | 0.04 | 0.30 | 0.35 |
| Custody (10× Python) | 0.08 | 0.11 | 0.51 | 1.21 |
| Fusion request | 0.01 | 0.01 | 0.10 | 0.10 |
| Tax engine (today's compute) | 0.06 | 0.15 | 0.60 | 1.55 |
| Render and joins | 0.02 | 0.02 | 0.15 | 0.20 |
| Persistence and commit (deferred-checkpoint floor plus 0.15 at 20k) | 0.15 | 0.17 | 1.40 | 1.60 |
| **Target end to end** | **≤ 0.45** | **≤ 0.60** | **≤ 3.2** | **≤ 5.2** |
| Python end to end | 1.80 | 2.30 | 12.81 | 21.65 |
| **Speed-up** | **≥ 4.0×** | **≥ 3.8×** | **≥ 4.0×** | **≥ 4.1×** |
| Compute only: custody through rendering and joins, Python / target | 1.09 / 0.17 | 1.49 / 0.29 | 8.53 / 1.36 | 16.70 / 3.06 |

- The daemon path drops start and open: Python 12.4 s against 3.05 s at
  generic 20k.
- SQLCipher books add the encrypted write floor (2.74–3.08 s for entries and
  postings at 20k, against 1.25–1.61 s plaintext) to both runtimes. Their
  budget is the plaintext budget plus that difference.
- With custody at 10×, the original budget makes the tax engine the largest
  compute line at 20,000. The separate speed pass now measures 0.299 s for
  generic and 0.439 s for multi-wallet `ledger` at that size. Integrating
  those savings into the complete journal pipeline still needs measurement.
- End-to-end 10× is out of reach under parity. Even with zero compute, the
  floors for generic 20k (open 0.14 + reads 0.11 + writes 1.25) give at most
  about 8.5×.

### Guarding the budget

- **Deterministic counters, in CI.** A `timing`-feature test build counts
  JSON parses and serializations, `Value` allocations on the production path,
  prepared statements and statement executions per table. The gate: zero JSON
  text on the production path except refusal fallbacks, and
  statement executions linear in rows with fixed per-table preparation. These
  checks do not depend on machine speed.
- **Ratio gate, in CI.** The generic 2,000 book is built once per job. Python
  and Rust `journals process` run back to back, best of three, and the job
  fails when Rust exceeds 0.35× Python, or when custody's in-process span
  exceeds 0.15× Python's custody phase. Ratios absorb runner speed; the
  margins absorb noise.
- **Local 20,000 gate, before each step's commit and before R4-16.** The
  bench compares against the stored baseline and refuses to record while the
  1-minute load average is above 1.5. It runs Python and Rust back to back.
  A line more than 10% over budget needs a written reason in the step record.
  Results are appended as JSON lines next to the checkpoint evidence.
- The bench books are synthetic, generated from committed seeds. Owner-book
  timings stay local, as owner-book comparisons do today.

## Bench harness (R4-1)

Four tools time a step, and the step record keeps what they report.

- **Bench books.** [`scripts/journals-bench-books.py`](../../scripts/journals-bench-books.py)
  builds a book with this checkout's CLI from a committed seed: `generic`
  (seed 1) or `multi-wallet` (seed 7), at any size. Its CSVs are
  byte-identical to the books the design measured. The multi-wallet book pairs
  half of its pegs, at most 300, with `transfers pair`, which authors custody
  components; at 20,000 transactions that takes about 15 minutes. The book
  ends processed, and `bench-book.json` records its shape, seed and CSV
  digests.
- **Python profile.** [`scripts/benchmark-journals-phases.py`](../../scripts/benchmark-journals-phases.py)
  times Python's phases on a fresh copy of the book and writes the raw
  snapshot. `--rust-runs 0` skips its binding comparison.
- **Rust bench.** `crates/journals` holds `kassiber_journals::compute`:
  custody, fusion and the tax engine in one process, which no command calls
  yet. Until fusion is typed (R4-8) it reads the finalized projection, the
  profile and the wallets in the JSON model the comparison harness reads, and
  a test proves both build the same ledger request and response. The
  `journals-bench` example (`cargo build --release -p kassiber-journals
  --features timing --example journals-bench`) times `compute` on a snapshot,
  with custody's spans and its own (`fusion.input`, `fusion.request`,
  `fusion.text`, `engine`). `--oracle` also writes and hashes the custody
  response text, which only the parity harness needs. `--check` compares the
  pipeline's fusion with `custody_with_fusion` over request files. Release
  builds never enable `timing`.
- **Recorder.** [`scripts/journals-bench-record.py`](../../scripts/journals-bench-record.py)
  runs Python and Rust back to back on one book and appends a record.
  `--rust-before` adds the previous step's bench to every pair, so a step's
  before and after come from the same minutes; every Rust run must report
  the same ledger hashes. It waits while the 1-minute load average is above
  `--max-load`. After `--wait-minutes` it measures anyway with at least five
  pairs and marks the record `loaded`.

### Bench records

A record is one JSON line with schema `kassiber.journals-bench.v1`:

| Field | Contents |
| --- | --- |
| `step`, `note`, `commit`, `dirty`, `recorded_at` | What was measured, from which checkout, and when |
| `machine` | CPU count, platform and Python version |
| `book` | Name, kind, transactions, seed, CSV digests and paired pegs from `bench-book.json` |
| `policy`, `max_load`, `loads` | `quiet` or `loaded`, the load cap and the load average before each pair |
| `python` | End-to-end time per pair and for the best pair, its command time, imports, result counts, snapshot size, and every phase of the best pair keyed by path |
| `rust`, `rust_before` | Compute time per pair and for the best pair, every span of the best pair, the fusion outcome and the SHA-256 of the ledger request and response |
| `lines` | The comparable lines below, for Python, Rust and the previous step |

The lines map the profile onto the pipeline. Python's paths are under
`dispatch/process_journals`; lines Rust does not run yet show no Rust time.

| Line | Python phases | Rust spans |
| --- | --- | --- |
| Snapshot in | `build/snapshot_load` (SQL) | `parse` (the JSON adapter until R4-9) |
| Custody | `build/custody_compute` | `admission`, `boundary`, the six stages |
| Fusion request | `request_build`, `request_dumps` | `fusion.input`, `fusion.request`, `fusion.text` |
| Tax engine | `engine_rust` | `engine` |
| Render and joins | `response_loads`, `response_render`, `balance_join`, `ownership_join` | — |
| Persistence | `store` | — |
| JSON parse and admission | — | `parse`, `admission` (the R4-2 target) |

Evidence of each run stays with the step's local verification.

### Recovered baseline and R4-2 target probe

`52b1e2b9` records the owner's decisions without an implementation change.
`ae79db2a`'s commit body reports unchanged corpus response hashes and kills
for its five moved `lib.rs` mutation recipes; it does not report a fresh
whole-catalog rerun.

The baseline file named above was absent at `ae79db2a`. The continuation
recovered all four R4-1 records before evaluating R4-2. The measured Rust
executables were built from that clean commit. Each book is a disposable
SQLite backup of the original design book, and every stored raw CSV row was
compared with the committed generator's seed output. The multi-wallet books
retain 171 and 300 active peg components. No original book was modified.

The owner's continuation instructions use a load cap of 8 and best of five
alternating Python/Rust pairs above that cap. Every record here used five
pairs, records all three load averages before each pair, and has stable Rust
ledger request/response hashes. All timings are observations under load.
Times below belong to the fastest complete run of each engine.

| R4-1 baseline, seconds | Python end to end | Rust compute | Rust parse | Rust admission | 1-minute load range |
| --- | ---: | ---: | ---: | ---: | ---: |
| generic 2,000 | 1.860 | 0.791 | 0.043 | 0.033 | 11.52–12.55 |
| multi-wallet 2,000 | 2.386 | 1.252 | 0.059 | 0.035 | 12.67–13.79 |
| generic 20,000 | 13.546 | 7.007 | 0.450 | 0.306 | 8.56–21.38 |
| multi-wallet 20,000 | 24.317 | 10.975 | 0.513 | 0.360 | 13.69–19.10 |

The multi-wallet 20,000 record's `dirty` flag reflects prototype edits made
while its saved baseline executable ran; its Rust executable and Python
implementation remained the baseline. The source `commit` on all four records
is `ae79db2a`.

The first R4-2 prototype used direct row visitors for `TxRow`, `WalletRef`,
`RateRow`, worklist rows and lossless SQL/transport values. It captured only
confirmation facts during admission and converted rows at the unported-stage
boundary. Its generic 20,000 probe alternated Python, saved baseline Rust and
prototype Rust for five pairs, at 1-minute loads 12.37, 14.35, 23.52, 38.53 and
43.81. The request and ledger response hashes agreed across both Rust binaries.

| Generic 20,000 probe, seconds | Baseline in the same pairs | Prototype |
| --- | ---: | ---: |
| Parse | 0.479 | 0.201 |
| Accounting admission | 0.100 | 0.097 |
| Confirmation capture | 0.111 | 0.001 |
| Rate/channel admission | 0.102 | 0.002 |
| Admission total | 0.312 | 0.100 |
| **Parse + admission (target ≤ 0.250)** | **0.791** | **0.301 — missed** |
| Explicit unported-stage boundary conversion | included in admission/stages | 0.304 |
| Full compute | 7.284 | 7.783 |

R4-2 stops at this failed target. Accounting admission still parses the
untyped `book_network.observations` population, taking 0.097 s; with this
parse cost it must fit within about 0.049 s. The additional 0.304 s boundary
conversion and retaining both typed inputs and unported stage operands also
make full compute slower. Re-planning should address accounting's network
facts and the lifetime/cost of boundary conversions before another full gate.
The prototype was preserved locally under `target/r4-2-prototype` and removed
from the shipped tree. Its later round-trip adapter and wrapper edits were
not measured or verified. No R4-2 parity, round-trip, replay, census/shadow or
mutation-kill acceptance is claimed, and R4-3 was not started.

The recovered prototype baseline contained **2,285** mutation recipes, rather
than the 2,246 recorded in the initial design. The discarded prototype re-anchored
four recipes (`REC-OPERATION-03`, `projection8-capture-before-rates`,
`projection8-row-order`, `projection8-confirmation`); its applicability check
found all 2,285 applicable, but no kill rerun was performed. Those recipe
edits were withdrawn with the implementation, so the shipped catalog is
unchanged. For this records-only continuation, the baseline executable builds,
all four benchmark runs, workspace formatting, the journal fusion tests and
preflight pass; inventory regeneration leaves the checked-in inventory
unchanged.

### Typed row and admission checkpoint (R4-2, partial)

The continuation starts at integrated R7 `dd433e4e` and takes the recovered
records from `4f663a62`, preserving the newer integration notes. It recovers
the archived row prototype and types the network observation inventory as well.
This is a checkpoint, **not R4-2/R4-3 acceptance**.

The shared types are exported by `crates/custody/src/snapshot.rs`:

- `snapshot/rows.rs`: `TxRow`, `WalletRef`, `RateRow`, `WorklistRow`,
  `NetworkRow`, `Column` and `SqlValue`. Direct visitors preserve missing
  versus null columns, storage classes, unknown columns and their order.
  Transport-only booleans, containers and wide integers are not silently
  coerced into SQL scalars. Signed and unsigned 128-bit visits from existing
  JSON values retain exact integers too.
- `snapshot/identifiers.rs`: `InternedId`, backed by `Arc<str>`. Transaction,
  wallet, profile, workspace, account and asset identifiers share allocations
  within one parse. The pool is released on success or refusal; independent
  books do not share an identifier cache.
- `snapshot/network.rs`: typed network observations and lossless malformed
  container operands. Network admission reads only relevant embedded payload
  fields and skips unrelated commercial evidence. The complete raw inventory
  remains available to the v1 oracle.
- `snapshot/book.rs`: `BookSnapshot`, including the ordered v1 text adapter.
  `parse` and `run` use this model. Nullable optional fields remain optional.
  `Snapshot` remains the compatibility alias for the unported `StageSnapshot`;
  its transaction objects are built once at the explicit timed boundary.

Accounting, confirmation capture, rate admission and channel admission retain
Python's order. Confirmation capture holds only the captured facts, rather
than a second original-row population. Nonempty channel contexts still use
the existing interpreter. Admitted network facts are not copied into the
legacy stage operands because no downstream stage reads them. Native component
callers in reports and transfers retain their existing validation algorithms.

The existing `serde_json/raw_value` feature retains the wire order of unported
fields, which `serde_json::Value` maps cannot retain. It adds no package or
network capability and changes no lockfile dependency. Its feature review and
license credit are updated. Python production code is unchanged.

Components and evidence still run their original JSON algorithms. R4-3 must
consume the typed input rows directly, provide typed stage outputs and shrink
the boundary. No R4-3 speed or full-custody non-regression acceptance is claimed
by the implementation alone.

All four R4-1 bench books ran as five back-to-back alternating before/after
pairs, with Python first in every pair. The recorded 1-minute loads were
14.73–22.65, above eight, so these are best-of-five measurements under load.
The measured source was the dirty checkpoint before this commit, against the
saved integrated `dd433e4e` executable.

| Seconds at 20,000 | Generic before | Generic checkpoint | Multi-wallet before | Multi-wallet checkpoint |
| --- | ---: | ---: | ---: | ---: |
| Parse | 0.4835 | 0.2151 | 0.4857 | 0.2602 |
| Admission | 0.3476 | 0.0165 | 0.3515 | 0.0387 |
| Boundary conversion | 0.0000 | 0.2908 | 0.0000 | 0.3301 |
| components+migration | 0.0000 | 0.0000 | 0.8286 | 0.8315 |
| evidence | 0.3935 | 0.3826 | 0.4193 | 0.4387 |
| interpretation | 0.5139 | 0.5294 | 1.9394 | 1.9949 |
| gaps | 0.4705 | 0.4772 | 0.4696 | 0.4414 |
| quantity | 2.0533 | 2.0560 | 2.2551 | 2.1577 |
| finalized_projection | 1.4987 | 1.5391 | 1.3417 | 1.6167 |
| Custody total (admission through finalized) | 5.2775 | 5.2916 | 7.6053 | 7.8498 |
| Pipeline compute (also parse, fusion, engine) | 8.2256 | 7.8055 | 11.7030 | 11.6814 |

Generic parse plus admission is **0.2316 s**, meeting the 0.250 s row/admission
target. This does not close the step's acceptance: conversion still costs
0.291/0.330 s, and custody totals regress despite slightly faster whole-pipeline
compute. The 2,000-row pipeline totals are 0.8523 → 0.8371 s generic and
1.3074 → 1.2980 s multi-wallet. Typed components/evidence and the full
non-regression target remain open.

The integrated catalog starts with **2,292** custody recipes and now contains
**2,309**; no recipe was removed. Four recipes move to equivalent operations:

- `REC-OPERATION-03` still skips channel admission before components, using
  the typed admission entry point.
- `projection8-capture-before-rates` still moves rate admission before
  confirmation capture, preserving the same first-refusal counterexample.
- `projection8-row-order` reverses the row/confirmation pairs rather than
  the former vector of already copied rows.
- `projection8-confirmation` still replaces captured confirmation with null.

The seventeen new recipes cover typed column order, unknown columns, boolean
storage, REAL signs, wide integers, exact msat admission, book-local identifier
sharing, network conflicts/binding/epochs/malformed inventories, and signed
and unsigned 128-bit visits. A second-pass correction also prevents serde's
positional struct-array deserialization from turning a non-object embedded
payload into network facts; `R4-network-payload-object` kills that regression.
Exhaustive new-branch coverage remains an open acceptance audit.

<!-- R4-2 checkpoint gate results. -->

The final manual-review correction passes workspace formatting, clippy with
`-D warnings` and all **1,503 workspace tests**. Every recipe in a touched
source file was rerun: **192 custody, 6 reports and 25 transfers**. Each kill
set equals its touched-file catalog. The custody catalog totals **2,309**;
the whole-catalog integration rerun is still required. The mutation scratch
build and the 25 GB workspace verification build were removed after saving
their summaries.

The final binaries match the saved integrated custody baseline over **25,348
requests**: the twelve generators' 24,000 books and 1,030 permutations, plus
159 retained replay requests and 159 permutations. Both full and decision-only
response hashes match, including their stage results and refusals. The typed
`BookSnapshot` text round-trip gate compares all 25,348 request lines with
zero byte differences. Recorded Python/Rust replay independently passes all
159 requests and 159 permutations with zero differences. Frozen custody and
refusal vectors pass in the workspace suite; all four benchmark books retain
matching before/after ledger request and response hashes.

The vacuous fusion witness passes as a regression test and also rejects a
standalone probe with exit status 1 and `vacuous fusion probe`: the seed has
zero distinct active inputs while its companion has one. Its local log and
counts are saved as `target/r4-3-verify/vacuous-failing-process.{log,json}`.
`classify_tests update` leaves the inventory unchanged, and the required
preflight passes all **57 tests**. The broader per-seed activity audit below
is still open.

Both requested Python lanes ran with `KASSIBER_TEST_TAX_ENGINE=census`;
the census pass also enabled `KASSIBER_TEST_CUSTODY_ENGINE=census`, and the
shadow pass used `run-custody-shadow`. These are the complete lane results,
without deselecting their failures:

| Lane | Passed | Failed | Skipped | Passed subtests |
| --- | ---: | ---: | ---: | ---: |
| core-accounting census | 2,031 | 2 | 75 | 460 |
| core-accounting custody-shadow | 679 | 2 | 40 | 231 |
| reports-contracts census | 2,471 | 4 | 10 | 2,824 |
| reports-contracts custody-shadow | 1,871 | 4 | 10 | 146 |

The core-accounting census exposes an existing harness mismatch in
`tests/test_pyfix_review.py::test_split_payout_fails_closed`, for both
`taxable-EUR` and `carrying-value-LBTC`. The test monkeypatches Python's
finalization to split an already computed payout decision; that alteration is
absent from the raw snapshot Rust receives. Census compares the deliberately
altered Python refusal with Rust's unaltered computation. The checkpoint and
the saved integrated baseline produce identical custody bytes and ledger
request/response hashes on both captured inputs. No Python code or test
expectation is changed here.

Reproduce on a disposable temporary root (create the directory first):

```sh
TMPDIR="$PWD/target/r4-3-verify/tmp" \
KASSIBER_TEST_TAX_ENGINE=census KASSIBER_TEST_CUSTODY_ENGINE=census \
MATURIN_NO_INSTALL_RUST=1 CARGO_NET_OFFLINE=true \
mise exec rust@1.99.1 uv@latest -- uv run --locked python -m pytest \
  tests/test_pyfix_review.py::test_split_payout_fails_closed -q
```

The local capture/replay helper, two raw snapshots and structured baseline
comparison are retained under `target/r4-3-verify/payout-repro/` (helper:
`target/r4-3-verify/repro-injected-payout.py`). This mismatch must be resolved
in the harness before calling the full census/shadow gates green.

The required worktree-local `TMPDIR` also exposes four pre-existing absolute
path assumptions in `tests/test_tax_shadow_harness.py::SessionEndTest`:
`test_a_swallowed_difference_still_fails_the_session`,
`test_a_test_that_stubs_the_engine_runs_uncompared`,
`test_an_ld5_refusal_is_counted_and_fails_the_session`, and
`test_census_fails_an_engine_caller_outside_the_overlay`. Each test's child
session produces the expected status and route counts, but its test-file path
is normalized to `target/r4-3-verify/tmp/...`; the assertion expects `in /`
or `overlay (...): /`. Reproduce with the same environment above and the
selector `tests/test_tax_shadow_harness.py::SessionEndTest`. The temporary
root remains in this worktree as required; the tests are not patched or
deselected to hide the mismatch.

Exactly what remains before accepting this run:

- Implement R4-3's typed component/evidence inputs and outputs, consume the
  shared rows directly, and reduce their 20,000-row compute spans to at most
  one third of the recorded 0.33/1.15 s baseline.
- Remove the remaining row-conversion cost and satisfy full-custody
  non-regression against back-to-back R4-1 runs.
- Finish the new-branch mutation audit and the fresh per-seed distinct-input
  activity report for all twelve generators. Corpus hash equality alone does
  not close that activity requirement.
- Resolve the two harness issues above, repeat the required census/shadow and
  parity gates after R4-3, and run the whole mutation catalog in integration.

### R4-4 typed compilation probe (stopped)

The `t3code/r4-5-typed-gaps` continuation starts at the integrated `dd433e4e`
and replays `4f663a62` as `5012ab6f`, retaining the R7 notes when resolving the
two documentation conflicts. Its first R4-4 probe **misses both compute targets**.
The prototype is withdrawn; R4-4 remains open and R4-5 is not started.

The probe introduces stage-local `InterpreterCompilation`, quantity claims and
slices, quarantine, native audit and reconciliation records. It suppresses the
isolated diagnostic composition on the normal stage path, limits pair-scope
indexing to pair endpoints, and clones the released reconciliation context only
when needed. Producer rows, pairs and canonical observations still use the old
JSON model. This tests the compilation/output seam; it is **not a complete typed
input/producer implementation**. It defines no crate-wide row types and changes
no Python code, dependencies, storage or product authority.

The unchanged R4-1 recorder runs Python, saved baseline Rust and saved prototype
Rust back to back, five pairs per 20,000-row book with `--max-load 8` and
`--wait-minutes 0`. Both books are disposable SQLite backups; every stored CSV
row and each CSV file equals the committed generator output, and multi-wallet
retains 300 active peg components. The baseline executable is built from
`5012ab6f` with only an added timing span around its existing Python-model output
conversion. Source patches and executable digests stay in
`target/r4-45-verify/`. The committed
`probe records` retain load averages, hashes and all
spans; these are rejected-prototype measurements, not accepted R4-4 records.
Times use each executable's fastest complete run, as the recorder selects it.

| Interpretation, seconds | Generic 20,000 | Multi-wallet 20,000 |
| --- | ---: | ---: |
| Baseline compute, excluding outer conversion | 0.463346 | 1.780767 |
| Baseline outer conversion | 0.000008 | 0.000062 |
| Prototype compute, excluding both conversions | 0.389428 | 1.552213 |
| Prototype producer-output to typed-record conversion | 0.000001 | 0.000002 |
| Prototype typed-record to oracle/migration conversion | 0.000011 | 0.000018 |
| Required compute ceiling | 0.153333 | 0.473333 |
| Whole pipeline, baseline / prototype | 7.581066 / 7.520282 | 11.356035 / 11.202095 |
| 1-minute load range | 19.21–25.19 | 13.86–37.06 |

The generic prototype spends 0.195198 s in holds alone, above the complete
stage's 0.153333 s ceiling. Building the input indexes takes another 0.049774 s,
pipeline preparation 0.044928 s and Samourai compilation 0.030884 s. The outer
conversion is negligible on this book, which emits no interpreter claims.
The multi-wallet prototype spends 0.722127 s in pipeline preparation, itself
above that book's complete 0.473333 s ceiling, plus 0.276040 s in holds and
0.225525 s in recorded-component compilation. The loaded baseline is slower
than the earlier 1.42 s reference; this does not relax the requested ceiling.
Typing only returned records and suppressing diagnostic construction cannot
meet the target. The next attempt needs reusable typed row/confirmation/scope
facts for the holds detectors, lazy indexes for empty derivations, and borrowed
rows with sparse overrides in pipeline preparation. Measure these kernels
before completing the output model; their internal conversions remain part of
this prototype's compute, not its small outer-boundary measurement.

The prototype's existing frozen interpretation selection passes 276 Rust tests
(139 module tests and 137 integration tests). Its edits touch files owning 101
mutation recipes: 17 in `mod.rs`, 51 in `pairs.rs` and 33 in `safety.rs`.
Two anchors need equivalent replacements, retained only with the local probe:

- `REC-SAFETY-03`: retain the remove-to-insert mutation through the lazy
  released-context accessor, so reconciled anchors fail to be released.
- `REC-STAGE-05`: empty the typed `native_component_reconciliations` vector,
  preserving the original certificate-omission mutant.

With these two replacements all **2,292** integrated recipes apply. The
catalog does not shrink. No kill rerun or new-branch mutation acceptance is
claimed. All prototype source and recipe changes are withdrawn at the target
stop; the committed implementation and catalog remain those of `5012ab6f`.
The complete generated activity/vacuous/refusal campaign, retained replay,
census/shadow shards and mutation kills must be rerun for a future accepted
implementation. A successful frozen selection and benchmark ledger hashes do
not substitute for those gates.

Separate `--oracle --runs 1` checks on both 20,000-row benchmark snapshots
produce identical full custody-response, ledger-request and ledger-response
hashes between the saved baseline and prototype executables. Their outputs and
`oracle-summary.json` stay with the local probe evidence. This bounds the
observed parity to these two benchmark inputs.

At the restored records-only boundary, inventory regeneration leaves the
checked-in inventory unchanged, all 57 preflight tests pass, workspace
formatting passes, and all 2,292 original mutation recipes apply. Full workspace
Clippy and workspace tests were not run for this withdrawn prototype; they
remain part of a future implementation's acceptance gates.

### R4-6 partial quantity target probe

R4-6 stopped for re-planning after a partial typed prototype missed the
quarter-time target. No quantity implementation or mutation-recipe change is
accepted by this checkpoint. The source baseline is `dd433e4e` plus the
R4-1/R4-2 records replayed as `7dad367a`. Python production source is unchanged.

The prototype introduced typed slices, decisions, postings, domains, projection
and eligibility indices, with a `CanonicalQuantityState` container. It retained
untyped canonical/evidence operands, native augmentation, reviewed compilation,
issues and reviewed economic terms; it is **not** the complete typed model in
the migration table. The finalization adapter explicitly materialized its
Python-model input under `quantity.boundary.finalized`.

The committed probe records use the R4-1 recorder and release `journals-bench`
with Rust 1.99.1 and locked Python 3.11. Both disposable 20,000-row books are
SQLite backups of the design fixtures; every CSV byte matches the committed
seed generator, and multi-wallet retains 300 active authored peg components.
Each measurement alternates Python, saved baseline Rust and prototype Rust
for five pairs because the 1-minute load exceeded 8. Values below are the
spans from each engine's fastest complete run; conversion is separate from
quantity compute. They are observations under load, not quiet-machine claims.

| 20,000-row book, seconds | Before compute | Before conversion | Prototype compute | Prototype conversion | Compute target |
| --- | ---: | ---: | ---: | ---: | ---: |
| generic-20000 | 1.736 | 0.218 | **1.209 — missed** | 1.060 | ≤ 0.395 |
| multi-wallet-20000 | 1.760 | 0.235 | **1.231 — missed** | 1.005 | ≤ 0.455 |

Full-pipeline compute rose from 7.729 to 8.193 s on generic and from 11.137
to 11.502 s on multi-wallet. Python end-to-end measured 13.723 and 23.592 s.
The respective 1-minute load ranges were 20.77–33.02 and 13.85–38.80.

The R4-1 recorder's existing custody aggregate does not include the prototype's
new `quantity.boundary.finalized` span. It must not be read as total custody
cost; the compute/conversion table and full-pipeline measurements include the
cost explicitly. Ledger request and response hashes agree in every paired run.

Generic quantity still spends 0.158 s on component compilation/indexing,
0.322 s on native augmentation and 0.428 s on arbitration. Multi-wallet spends
0.264, 0.248 and 0.414 s respectively. The unchanged generic native/compiler
work alone exceeds the generic compute budget. The new compatibility adapter
rebuilds serde trees, clones the canonical population and eligible decisions,
then converts that materialized tree to Python-model values. Its 1.005–1.060 s
cost is additional work, not a speedup from moving the old conversion span.

Re-planning must cover native/reviewed compilation and a shared typed
observation/index boundary, not just the arbitration output. The next adapter
must avoid rebuilding large serde objects before Python-model conversion;
the final wire serializer must write typed fields directly. Temporary
neighbour conversion must remain separately measured, and cannot be used to
hide generic stage computation or relax the target.

The rejected patch and saved binaries remain local under `target/r4-6/`.
The patch SHA-256 is
`4673ac644a91062fdbac95c0dce642225e63f2031a3c8d148b2b1336d22a07df`.
All three implementation files were restored byte-for-byte before this
records-only checkpoint. No prototype code supplies production results.

The prototype passes `cargo check`, formatting and 41 selected Rust tests,
including frozen quantity arbitration, exact amounts, compiler/runtime wrappers
and scoped barriers. Complete custody-response, ledger-request and
ledger-response hashes also match the baseline on both cached 20,000-row
snapshots. This is bounded parity evidence, not the full generated/replay gate.
The restored checkpoint passes workspace formatting and all 2,292 catalog
applicability checks. Immediately before the checkpoint commit, inventory
regeneration leaves `tests/rust_port/dispositions.json` unchanged and the
required `preflight --ci` shard passes 57 tests.

The live catalog contains 2,292 recipes. Twelve anchors moved in the discarded
prototype: `quantity-arbitration-posting-order`, `QRT-056`, `QRT-069`,
`QRT-102` through `QRT-109`, and `R2-review-finalized-allowlist`. They were not
re-anchored or run as mutants, and no kill is claimed. Restoring the code
restores their original anchors; the catalog does not shrink.

Remaining work is the complete typed quantity model and direct serializer,
repeated 20,000-row performance acceptance and the 2,000-row timings, the
24,000-book/1,030-permutation activity/vacuity campaign, retained replay, full
frozen vectors and exact stage refusals. Re-anchor rewritten recipes, add
witnesses for new branches and kill the full catalog. Core-accounting and
reports-contracts still need census and custody-shadow with tax census;
Clippy `-D warnings` and workspace tests also remain. R4-6 remains open.

### R4-6 quantity kernel follow-up

The quantity kernel checkpoint replaces
the partial typing approach with borrowed native events, shared observation
indexes, typed arbitration kernels and direct construction of the existing
Python-model output. Stage inputs and outputs remain unchanged. Total quantity
CPU falls by 4.3–4.8× across the generic and multi-wallet books at 2k and 20k,
including output construction. The new run uses process CPU as its primary
acceptance measure under load; wall time improves by 2.5–2.9×, so the earlier
quarter-time wall budget remains open. This does not complete the full typed
pipeline migration or establish whole-custody performance at 10× Python.

### R4-7 finalized-projection target probe

This run starts at integrated `dd433e4e`, with `4f663a62`'s baseline records
cherry-picked as `3cb1b899`. **R4-7 is incomplete.** Its typed prototype missed
both stage-compute targets and was withdrawn before the acceptance campaign.
The committed implementation change is only the opt-in `custody.total` span.
It covers `run()`, including operation-local memo cleanup, and excludes request
parsing, oracle response encoding and fusion. It contains the individual stage
spans; do not add it to their sum. Without `timing` it remains a no-op.

The prototype introduced `FinalizedTaxProjection`, typed finalized quantities
and generated row ids, intra/cross-asset pair records, and quarantine records
with lossless Python payloads. A small raw-row accessor shared source records
through `Arc`; historical SQL fields remained in their raw representation.
It moved output conversion to the legacy fusion boundary. Observation and
decision selection still used generic records, so this was not a completed
field-based implementation of the stage. No typed API from that prototype is
shipped or available for R4-8 to borrow.

The two `R4-7-probe` records use the R4-1
recorder, with five alternating Python/baseline/prototype pairs per book,
`--max-load 8 --wait-minutes 0`. The one-minute loads were 17.85–25.46 for
generic and 14.15–41.59 for multi-wallet. Every CSV byte matched the committed
generator, and SQLite backups of the synthetic design books were made under
this worktree's `target/r4-7/books`; the multi-wallet backup has 300 components.
The saved baseline executable was built before prototype edits. All ten pairs
retained the same ledger request and response SHA-256 for each book.

Times below are seconds from the fastest complete run of each executable,
rather than independently choosing the fastest stage from different runs.

| 20,000 transactions | Generic before | Generic prototype | Multi-wallet before | Multi-wallet prototype |
| --- | ---: | ---: | ---: | ---: |
| Finalized input conversion | 0.246 | 0.320 | 0.255 | 0.333 |
| Finalized compose (before includes output conversion) | 0.874 | **0.525** | 0.903 | **1.135** |
| Projection-to-Python conversion (prototype: at fusion) | included above | 0.644 | included above | 0.566 |
| Remaining finalized-stage elapsed | 0.212 | 0.257 | 0.228 | 0.263 |
| Complete finalized-stage span, excluding fusion | 1.332 | 1.103 | 1.385 | 1.731 |
| In-process `custody.total` | not instrumented | 4.656 | not instrumented | 7.809 |
| Full pipeline compute | 7.512 | **8.135** | 11.241 | **12.395** |

Typed compute misses the requested **0.2625 s generic / 0.290 s multi-wallet**
limits. The output conversion is no longer inside `custody.total`, but remains
necessary for the untyped fusion caller; moving that cost does not establish
a custody speedup. The full pipeline regresses on both books.

The remaining generic input maps, repeated field conversions and selection
indexes still cost too much. The multi-wallet path additionally materializes
all finalized rows back into JSON when any reviewed conversion exists, because
the conversion matcher remained untyped. Raw-row copying at the input seam and
full output materialization at fusion also preserve substantial conversion and
destruction costs. Re-planning must replace these hot paths with typed
observation/decision/slice operands and a conversion matcher borrowing typed
rows, and make the legacy output conversion single-pass. The next attempt
must measure those changes before claiming either stage or custody acceptance.

The supplied sampler collected 393 main-thread samples of three generic
prototype runs after allowing tracing of the owned benchmark process. Generic
cloning, destruction and output conversion remain visible; 52.9% of self
samples are unresolved in this optimized build, so the sample is not a precise
per-function cost attribution. The first sandboxed attempt collected no
samples because `ptrace` was denied.

The rejected source, patch, saved executables, raw benchmark evidence and
profiling artifacts remain locally under `target/r4-7/`. They are exploratory
artifacts, not a verified implementation checkpoint. The live integrated
mutation catalog has **2,292** entries (the older baseline record's 2,285 was
not this integrated tip). It does not shrink. No recipe was re-anchored or
added in the committed instrumentation change.

The rejected prototype passed 34 focused Rust projection tests: frozen
helper/adapter outputs, lossless diagnostics and refusal witnesses. These do
not substitute for the complete acceptance campaign.

The final instrumentation-only checkpoint passes workspace formatting,
workspace/all-target Clippy with `-D warnings`, all **1,493 Rust workspace
tests** (including the owned loopback-server tests), and the required CI
preflight (**57 tests**). The required inventory update leaves its checked-in
file unchanged. A timing-enabled generic 2,000 run emits `custody.total` once
and preserves the saved baseline's full custody-response, ledger-request and
ledger-response hashes.

All **seven** recipes in the only touched Rust file, `lib.rs`, are killed
after three named baseline selections pass: `rates-before-components`,
`REC-OPERATION-01` through `REC-OPERATION-05`, and
`projection8-capture-before-rates`. The kill ids equal that file's catalog;
all **2,292** catalog entries still apply. No full-catalog kill rerun is claimed.
Mutation and workspace verification build trees were removed after writing
their summaries.

Remaining R4-7 work:

- Complete the typed compiler and its borrowable `FinalizedTaxProjection` API,
  including typed reviewed conversions and input selection, then meet both
  compute limits with input/output conversion reported separately. Recheck
  the aggregate custody target after all four stage workers are integrated.
- Run the twelve generators (24,000 books and 1,030 permutations), with
  per-seed distinct-input activity and a deliberately vacuous control that
  fails. Compare every stage and whole-pipeline byte, frozen vectors and exact
  refusals, and all recorded replay inputs and permutations, including fresh
  recordings.
- Re-anchor every rewritten mutation recipe, add recipes for new branches,
  list the equivalents, and rerun the required catalogs without shrinking
  them. Run census and custody-shadow on `core-accounting` and
  `reports-contracts` with `KASSIBER_TEST_TAX_ENGINE=census`. Repeat workspace
  fmt/Clippy/tests, inventory update and CI preflight on that implementation.

This target miss stops the implementation run; no R4-7 acceptance or typed
fusion handoff is claimed.

### Projection and fusion kernel checkpoint

Local branch `t3code/custody-perf-projection` starts at `dd433e4e` with the
R4-1 records and R4-7 instrumentation cherry-picked as `cf2d4a1b7` and
`25b87bfc8`. This pass retains every stage's existing input/output model and
the request-text route into the ledger. It does not deliver the typed fusion
handoff in the original R4-8 row. **Both performance targets remain unmet.**
The implementation checkpoint is `78df09179`.

The implementation removes work within the existing kernels:

- Finalized composition borrows observations and decisions instead of cloning
  them. Observation sort keys are cached, and barrier event order is built
  only when barriers exist. Completed JSON strings and containers move into
  the existing Python-value output instead of being copied again.
- The runtime selects only observation, decision and event fields that the
  compiler reads. It still checks the complete supplied value for lossless
  admission, including unused fields; uncertain operands retain the exact
  original round trip and refusal order.
- Fusion retains the decimal values already admitted by its invariant pass,
  defers Python repr construction until a refusal needs it, moves completed
  values into request maps, and uses the plain-string fast path for `str()`.
- A conservative graph-absence proof skips physical hold detection only for
  graphless `custody-tax:` slices. Other synthetic prefixes, graph arrays,
  malformed raw JSON and raw hex retain the original detectors. Asset groups
  keep their first-seen order, including groups containing only skipped rows.
- Request serialization writes the existing ordered schema directly. Wide
  integers and floats still use the CPython scalar codec, preserving exact
  engine refusal text and its wire positions.

#### CPU and wall measurements

`target/projection-perf/pairs.jsonl` contains five alternating before/after
pairs for each book in each clock mode: 80 process runs in total. Both saved
executables use Rust 1.99.1 release builds. Normal runs record wall spans;
separate runs preload a scratch-only `clock_gettime` adapter that maps Rust's
monotonic clock to `CLOCK_PROCESS_CPUTIME_ID`, measuring user plus system CPU
across all threads. Child `getrusage` independently records complete process
CPU. No clock adapter enters product code or the binding.

The table uses the fastest complete pipeline run of each binary in each clock
mode, retaining all its spans together. Process CPU includes oracle encoding,
hashing and destruction; `compute` excludes that oracle work. Nested spans
overlap and must not be summed. Every pair preserves the custody response,
ledger request and complete ledger response SHA-256. These synthetic snapshots
are the inherited performance books, rather than fresh Python timing runs.

| Book | Span | Before CPU / wall (s) | After CPU / wall (s) |
| --- | --- | ---: | ---: |
| generic 2k | finalized input | 0.1322 / 0.0394 | 0.0747 / 0.0209 |
| generic 2k | finalized compose | 0.1474 / 0.1276 | 0.0677 / 0.0760 |
| generic 2k | finalized stage | 0.3065 / 0.2065 | 0.1757 / 0.1224 |
| generic 2k | fusion input | 0.0972 / 0.0521 | 0.1030 / 0.0703 |
| generic 2k | fusion request | 0.1575 / 0.1226 | 0.0485 / 0.0426 |
| generic 2k | fusion text | 0.0445 / 0.0310 | 0.0152 / 0.0146 |
| generic 2k | engine | 0.0918 / 0.0908 | 0.0807 / 0.0804 |
| generic 2k | custody total | 1.3015 / 0.7647 | 1.0722 / 0.5532 |
| generic 2k | compute | 1.7636 / 1.1340 | 1.4026 / 0.8343 |
| generic 2k | process | 2.1263 / 1.3708 | 1.7631 / 1.0308 |
| multi-wallet 2k | finalized input | 0.1573 / 0.0335 | 0.0873 / 0.0168 |
| multi-wallet 2k | finalized compose | 0.1630 / 0.1131 | 0.0642 / 0.0590 |
| multi-wallet 2k | finalized stage | 0.3435 / 0.1800 | 0.1708 / 0.0952 |
| multi-wallet 2k | fusion input | 0.0813 / 0.0498 | 0.1090 / 0.0654 |
| multi-wallet 2k | fusion request | 0.1255 / 0.1535 | 0.0440 / 0.0438 |
| multi-wallet 2k | fusion text | 0.0300 / 0.0359 | 0.0155 / 0.0148 |
| multi-wallet 2k | engine | 0.1795 / 0.2429 | 0.1983 / 0.1896 |
| multi-wallet 2k | custody total | 1.9113 / 0.9225 | 1.7514 / 0.9134 |
| multi-wallet 2k | compute | 2.4097 / 1.4833 | 2.1981 / 1.3112 |
| multi-wallet 2k | process | 2.7424 / 1.7264 | 2.6289 / 1.4744 |
| generic 20k | finalized input | 1.5912 / 0.2639 | 0.7258 / 0.1464 |
| generic 20k | finalized compose | 1.7616 / 1.2599 | 0.6935 / 0.6165 |
| generic 20k | finalized stage | 3.6747 / 1.8085 | 1.6327 / 0.9632 |
| generic 20k | fusion input | 0.6770 / 0.3953 | 0.8184 / 0.5273 |
| generic 20k | fusion request | 1.0654 / 0.8421 | 0.4274 / 0.5161 |
| generic 20k | fusion text | 0.2836 / 0.1940 | 0.1439 / 0.1611 |
| generic 20k | engine | 0.7385 / 0.8055 | 0.7591 / 0.9346 |
| generic 20k | custody total | 12.8663 / 6.4674 | 11.2254 / 5.0284 |
| generic 20k | compute | 16.2562 / 9.3678 | 14.0516 / 7.8472 |
| generic 20k | process | 19.1004 / 10.8117 | 16.8343 / 9.2510 |
| multi-wallet 20k | finalized input | 1.3759 / 0.2455 | 0.7208 / 0.1310 |
| multi-wallet 20k | finalized compose | 1.6120 / 1.0377 | 0.6587 / 0.7244 |
| multi-wallet 20k | finalized stage | 3.2369 / 1.5565 | 1.5618 / 1.0726 |
| multi-wallet 20k | fusion input | 0.6819 / 0.4147 | 0.8959 / 0.4875 |
| multi-wallet 20k | fusion request | 1.4170 / 0.7931 | 0.4438 / 0.4191 |
| multi-wallet 20k | fusion text | 0.3200 / 0.1968 | 0.1342 / 0.1322 |
| multi-wallet 20k | engine | 1.9020 / 1.9779 | 2.0798 / 2.0117 |
| multi-wallet 20k | custody total | 16.9069 / 8.0735 | 15.2602 / 7.4156 |
| multi-wallet 20k | compute | 21.9168 / 12.1418 | 19.5237 / 11.1495 |
| multi-wallet 20k | process | 24.8033 / 13.5524 | 22.4439 / 12.6048 |

One-minute load ranges were 32.45–36.20, 33.89–35.95, 27.73–33.89 and
22.36–29.02 for those four books, respectively. The full finalized stage
saves 52–56% CPU at 20k, and the request builder saves 60–69%. This does not
meet projection at one quarter of baseline or the 0.10 s request budget.

Five additional CPU pairs per 20k book reverse executable order. Loads are
17.46–21.45. Best individual engine spans still increase: generic
0.7110 → 0.7655 s and multi-wallet 1.8119 → 1.9083 s. Engine code and request
bytes are unchanged, but the no-engine-slowdown gate is not established.
The unchanged fusion input text round trip also costs more CPU in this
process layout. Neither increase is hidden in the kernel savings.

Two preliminary follow-ups were removed. Distributing owned output conversion
over worker threads increased projection CPU substantially and did not remove
the engine increase. Compiled field selectors and additional scalar fast paths
did not show a consistent aggregate CPU improvement across the two 20k books.
Their probe logs remain under `target/projection-perf/`; neither is in
the implementation checkpoint.

#### Verification and remaining work

The complete generated campaign compares baseline and optimized custody
response bytes, independently recomputes Python stages and fusion, and
compares the complete ledger response bytes: **24,000 books plus 1,030
permutations, zero differences**. The per-suite activity counts match the
[fusion campaign](custody-engine.md#fusion-generated-campaign-and-bounded-mutation-rerun):
16,227 active distinct seed inputs, with every seed's own count retained in
`target/projection-perf/activity.jsonl`. A deliberately vacuous seed is
rejected even when its companion has an active request. Read-only retained
replay passes 159 requests plus 159 permutations, including byte comparison.
The 311 frozen builder/fusion vectors include 67 builder refusals and 30 exact
engine refusals across all six refusing ledger stages.
Direct baseline/current probe output also matches for all 311 fusion vectors,
including the 67 builder refusals, and all 243 complete ledger response strings.
The same control matches 340 projection primitive vectors and 17 raw projection
vectors. Rust frozen adapter tests additionally compare encoded result bytes,
including the lossless diagnostic seam.

Eight recipes move to equivalent mutants:

| Recipe | Equivalent mutation at the new anchor |
| --- | --- |
| `projection8-observations` | Replace the selected observation input with an empty list |
| `projection8-decisions` | Replace the selected decision input with an empty list |
| `projection8-events` | Replace the selected event input with an empty list |
| `FUSION-FIELD-rate` | Reverse exact/fallback rate precedence in the retained invariant result |
| `FUSION-OMISSION-chain` | Rename the emitted `chain_order` field so the contract field is absent |
| `FUSION-WIRE-ORDER` | Reverse schema-key traversal |
| `FUSION-WIRE-FIELD` | Change Unicode escaping to `ensure_ascii(true)` |
| `FUSION-WIRE-OMISSION` | Skip the first schema key |

Two new graphless-hold recipes preserve synthetic-prefix scope and the
conservative graph gate. The catalog grows from **2,292 to 2,294**.
All **143** recipes in the five touched Rust files build and are killed:
65 composition, 32 quarantine, 25 runtime, 12 fee and 9 fusion recipes.
The 26 unmutated test groups pass first. The fresh kill-ID set exactly equals
the selected catalog, with no missing, extra or duplicate kills; all inherited
recipe IDs remain in the full catalog. This does not claim a fresh rerun of
all 2,294 recipes. The single mutation build tree is removed after the run.

Core-accounting census completes with 2,031 passes, 75 skips and 460 successful
subtests, plus two failures in `test_pyfix_review.py::test_split_payout_fails_closed`
(taxable/EUR and carrying-value/LBTC). Both failures reproduce against the saved
baseline binding in fresh census and shadow controls. These tests monkeypatch
Python's finalized compiler to split already-loaded decisions; Rust receives
the unmodified raw snapshot, so the
comparison harness expects a Python-only injected refusal from an unchanged
Rust input. The run compares 260 custody snapshots and 254 fusion outcomes.
This records an existing harness mismatch, rather than a passing census gate.

Core-accounting custody-shadow likewise reports those two failures, with 679
passes, 40 skips and 231 successful subtests. It compares 254 custody snapshots,
252 frozen Python results and 248 fusion outcomes. Reports-contracts census
reports 2,471 passes, 10 skips and 2,824 successful subtests, with four harness
self-test failures caused by an in-checkout temporary directory rendering
relative paths where the assertions expect absolute paths. All four pass when
rerun with an external disposable temporary root. The full reports census
compares 224 custody snapshots and 196 fusion outcomes; its original command
is not recorded as green. Subsequent checks use the external disposable root.
Reports-contracts custody-shadow passes: 1,875 tests, 10 skips and 146 successful
subtests; 217 custody snapshots, 214 frozen Python comparisons and 194 fusion
comparisons, with zero differences.

Rust formatting, workspace/all-target clippy with `-D warnings`, and workspace
tests pass (1,495 tests, zero failures). All 2,294 catalog recipes apply, and
all 145 ledger feature-contract outputs remain byte-identical with and without
custody's serde features. `classify_tests update` produces no inventory diff.
The required preflight shard passes: 57 tests and 60 successful subtests.

The final SHA and fresh generated/replay/frozen byte-check results are retained
in `target/projection-perf/final-verification.json`, alongside the raw paired
measurements, baseline binary hashes, `mutation-audit.json`, shard logs and
individual gate results. Comparison helpers and disposable test data stay
outside product code. Build trees are removed after their summaries are saved.

Remaining CPU is in generic field selection, per-row JSON construction/destruction,
timestamp rendering and the retained fusion input round trip. The 240-sample
fusion profile still
shows map indexing and allocation throughout the row constructor and
invariant pass; 43.8% of self samples are unresolved, so it is not a precise
cost attribution. Finish the performance targets and investigate the engine
and fusion-input increases before claiming R4-7/R4-8 acceptance.

## Authority switch (R4-16)

### Before the switch

- Every stage, fusion, rendering and persistence comparison is green on all
  corpora, and the whole mutation catalog, including the new R4 entries, is
  killed.
- The CLI subprocess suites and the daemon smoke sessions pass with
  `KASSIBER_TEST_CLI` naming the hybrid candidate, and the hybrid log shows
  `journals process` (and `ui.journals.process` with
  `KASSIBER_HYBRID_RUST_DAEMON=1`) served by Rust.
- Readiness parity: after a Python build and a Rust build from the same
  starting copy, every report-readiness and freshness payload matches (stored
  freshness markers, quarantines, component blockers, quantity
  `report_blocked`, filed-report impact resolution, and the final-export
  `blocking_reports` flag).
- Failure injection before and after each durable step, including a refusal in
  every stage and a panic: the database is unchanged and the markers stay
  stale.
- The performance budget holds on the local 20,000 gate.

### What switches

- The registered Rust CLI and daemon journal paths become production authority
  only after these gates pass. R4-13/R4-15 registration supports development
  hybrid verification before the switch. Production Rust builds have no Python
  bridge: a native refusal is the answer. Development hybrid replay requires
  proof of unchanged durable state and records every refusal shape for the
  lossless follow-up.
- The Python product is not changed. The desktop keeps the Python sidecar
  until the R7 cutover, so users get the speed-up at cutover. Decision 1
  rules out an early path through PyO3.

### Coexistence with Python on one book

- **Shared invalidation.** `journal_input_version` is bumped by schema
  triggers and by explicit `invalidate_journals` calls, so writes from
  either runtime invalidate projections built by the other. Both write
  `last_processed_at`, `last_processed_tx_count` and
  `last_processed_input_version` the same way. `projection_freshness`
  evaluates identically.
- **One writer.** Both take `BEGIN IMMEDIATE` before reading. A Python
  freshness worker and a Rust CLI build serialize on SQLite's writer lock,
  under the same busy timeout. Neither can commit a projection read from an
  older input version.
- **Ownership.** The Rust command takes the same role lease as Python's
  `journals process` (R1 ownership compatibility). Only one daemon of either
  runtime owns a project, so the Python and Rust freshness queues never run
  against one book at once.
- **Events.** A CLI build, in either runtime, emits no daemon events. A
  running desktop learns about it on its next freshness or readiness read,
  as it does today.
- **Byte-compatible state.** A Python build after a Rust build, or the
  reverse, replaces every row. No stored value records which runtime built
  it. The engine's `"engine": "kassiber_tax"` metadata is unchanged because
  the engine is the same.

### Rollback

Nothing to migrate. Stop routing to Rust (the hybrid candidate, or at R7 the
packaged sidecar), and the next Python `journals process` rebuilds every row.
To force that without waiting for an input change, bump `journal_input_version`
(`invalidate_journals`). Readiness then reports the book stale until the
rebuild. Filed snapshots and saved exports are never rewritten, as today.

### What stays Python

Within the owner's scope (R4 and R5 now, R6 and R7 untouched):

- The AI chat's tool projections, including the assistant's
  `ui.journals.process` tool (which rebuilds without pairing), MCP and the
  operator broker stay Python under R6. Their results come from Python's
  journal step until R6 ports them. Both runtimes produce the same stored
  state, so reports read either.
- The Python reference implementation stays as the oracle for every
  comparison until R7, and as a test-only reference after it.
- Plain `journals process` needs no network. Automatic pairing, pricing and
  the freshness job read only local data; R4 adds no egress.

## Risks

- **Catalog churn.** Typed rewrites invalidate mutation anchors across about
  25,000 lines. Re-anchoring is manual, and an equivalent mutant can hide a
  lost check. Mitigation: the full catalog rerun every step, kill set equals
  catalog, and reviewed equivalence notes for any change of count.
- **Storage-class fidelity.** Historical books store some columns as REAL or
  TEXT where current ones store INTEGER, and Python's behavior follows the
  stored class. `SqlValue` keeps the class; the historical fixtures and the
  loader's v1 byte gate cover it. A concrete type is used only where the
  compatibility pass guarantees the class.
- **Three lines to join.** The custody performance branch and the fusion
  branch both start at `1b62f405`. The R3/R5 storage stack forked from the
  custody line at `6da3e459` and later took custody only up to `a4e017a3`
  (R2c channel lifecycle). R4-0 is a real merge with its own full gate.
- **Intermediate slowdowns.** Boundary conversions while neighbors are
  untyped can make a step slower than its predecessor on some book.
- **Allocator contention.** A gdb sample of Rust custody at 20,000
  (`pmp-20000-7`) put about 15% of main-thread samples in glibc's
  `__lll_lock_wait_private`/`__lll_lock_wake_private`, consistent with malloc
  arena contention under the eight-thread map. Typed data allocates less.
  Changing allocator would be a dependency change and needs approval.
- **Write floor.** I/O-bound writes dominate at 20,000. Checkpoint deferral
  must not let the WAL grow without bound when processes are killed between
  commit and checkpoint; the next open's automatic checkpoint covers that.
- **Engine cost.** The engine is outside custody's typed rewrite. The
  separate speed pass reduces its text-in/text-out cost to 15–22 µs per
  transaction on the 20,000 books; the fused pipeline still needs its own
  measurements.

## Storage-format speedups

The owner approved all three of decision 4 on 2026-10-30: compact retained
calculations, deterministic entry ids with changed-row writes, and a faster
SQLCipher write mode. Python implements them first as the reference, one
commit each, and R4-12 mirrors that behavior. Every report, export and AI
projection keeps its values. The visible changes are the entry ids, the
order among entries with the same timestamp, and `created_at`. They are
listed under each speedup.

### Prototype measurements

A prototype in the local scratch tooling replays generic 20,000's stored rows
(20,000 entries, 40,000 quantity postings). It times only the statements
`store_ledger_state` would run on each table, plus `COMMIT`, through Python's
drivers with the connection settings of `kassiber/db.py`:

- plaintext uses WAL/NORMAL on SQLite 3.53.1;
- SQLCipher uses DELETE/FULL on `sqlcipher3` 0.6.2 (SQLCipher 4.12.0,
  SQLite 3.51.1).

Each run starts from a fresh copy whose state is what the previous build
left. The figures are best of five; variants ran round-robin because the
1-minute load stayed between 11 and 32.

| Write, seconds | Plaintext | SQLCipher |
| --- | ---: | ---: |
| `journal_entries` today: delete, then insert 20,000 uuid4 rows | 0.87 | 2.28 |
| … compact calculations | 0.67 | 1.47 |
| … 128 MiB rebuild page cache | 0.74 | 1.48 |
| … compact calculations and 128 MiB cache | 0.58 | 1.07 |
| … deterministic ids, nothing changed (read and compare only) | 0.14 | 0.29 |
| … compact, nothing changed | 0.11 | 0.19 |
| … compact, 1% of entries appended | 0.14 | 0.26 |
| … compact, 5% of entries changed | 0.33–0.36 | 0.56 |
| `journal_quantity_postings` today: delete, then insert 40,000 rows | 0.51 | 0.98 |
| … nothing changed (read and compare only) | 0.15 | 0.27 |
| Reference: SQLCipher WAL/NORMAL, full entries rewrite | | 1.87 |
| Reference: SQLCipher WAL/NORMAL with 128 MiB cache, compact | | 0.95 |

- Python's persistence plus commit at generic 20,000 is 2.25 s today, of
  which the two tables above are about 1.4 s plaintext and 3.3 s SQLCipher.
- In a rebuild that changes little, both tables cost 0.25 s plaintext and
  0.46 s SQLCipher.
- A full rewrite drops from 1.38 s to 0.9 s plaintext and from 3.3 s to
  about 1.9 s SQLCipher. A full rewrite happens on a fresh book, on the
  first build after the upgrade, and when an early change moves later lots.
- Peak memory during the SQLCipher full rewrite rose from 121 to 173 MB with
  the 128 MiB cache.

### 1. Compact retained calculations

**On disk.** `journal_entries` gains a nullable `calculation_compact TEXT`.
A realized entry's engine calculation is stored there as one positional JSON
array (`[2, engine, method, sources, fragments]`), specified in
`kassiber/core/journal_calculation.py`.
`calculation_json` is NULL on those rows.

- Each distinct event or lot is stored once and referenced by index. In every
  bench book, all fragments of an entry share one event.
- The twelve pricing values are positional.
- Each `*_msat_exact` string is restored as the string of its integer.
- A calculation the encoding cannot restore exactly keeps today's
  `calculation_json` text and a NULL `calculation_compact`. This covers an
  unknown key, a float, a mismatched exact string or a boolean. No row has
  both columns set.

Calculations shrink to 29–30% of their size: 37.2 → 10.8 MB at generic
20,000 and 37.7 → 11.4 MB at multi-wallet 20,000. That takes
`journal_entries` from 49.6 to about 23 MB.

**Rejected alternatives:**

- Compact separators alone save 6%.
- Compression: CPython's zlib and Rust's `miniz_oxide` emit different bytes,
  which breaks R4-12's database-dump parity, and compression costs CPU on
  every build.
- Sharing lots across rows (11,955 distinct lots for 19,988 references at
  generic 20,000): rows would depend on each other, which defeats
  changed-row writes and gives readers a join.

**Exactness.** The decoder returns what `json.loads` returns for the legacy
text: the same values, types and sorted key order. It builds a new object
for each reference, because the explanation writes `inherited_basis` into
each lot. Re-serializing a decoded value with `json.dumps(sort_keys=True)`
reproduces the legacy bytes. A check over every calculation in the five
bench books found every one representable and every legacy byte reproduced.
Encoding costs about as much as today's `json.dumps`: 0.31 s against 0.28 s
at 20,000 under load.

**Migration.** `ensure_column(journal_entries, calculation_compact, TEXT)`
runs after the msat rebuild, whose fixed column list would otherwise drop
it. A fresh schema places the column after `calculation_json`.

- Old rows are read from `calculation_json` as today. The next build writes
  compact rows.
- There is no conversion at open. The first build after the upgrade
  rewrites every row anyway (speedup 2), so converting at open would only
  add a second rewrite of about 37 MB at launch.

**An older binary with a newer book:**

- Older Python ignores the column.
- Its `reports explain-capital-gain` finds `calculation_json` NULL on compact
  rows and reports `engine_detail_unavailable`. That fails closed and invents
  no numbers.
- Its `journals events get` shows `calculation_json: null` and the extra
  `calculation_compact` field.
- Its next `journals process` writes legacy rows, which current readers read.
- An older Rust binary refuses the book because its schema manifest lacks
  the column. R3 refuses every newer schema the same way.

**Readers:**

- `retained_calculation`, used by the explanation and the inherited-carry
  chain, reads through `stored_calculation(row)`.
- `journals events get` returns `calculation_json` as the legacy bytes and
  omits `calculation_compact`, so its output is unchanged.
- `load_stored_ledger_state` passes the row to the exit-tax path, which
  reads neither column.
- No Rust reader, report, export, audit package, filed-report hash or AI
  tool reads calculation text. The explanation is a desktop and CLI read,
  not an AI or MCP tool.

**Risks:**

- Explanation output is the decoded value and stays byte-identical.
- Tests that edit `calculation_json` to simulate corruption must now edit
  the column the row uses.
- Backup and restore copy whole files. Restoring an older backup runs the
  migration on open.

**R4-12 mirror.**

- Write the same bytes: the compact array with `,` and `:` separators and
  Python's `ensure_ascii` escaping.
- Use the legacy fallback (`json.dumps(sort_keys=True)`, default separators)
  for any calculation the typed model could produce outside the checks.
- This commit already carries the store mirror: the schema manifest, the
  ordered pass and the fixtures.

### 2. Deterministic entry ids and changed-row writes

**On disk.** Entry ids and `created_at` are written as follows:

- **Id.** `journal_entries.id` is a UUIDv8 (RFC 9562) built from the first
  16 bytes of SHA-256 over
  `["kassiber-journal-entry-v1", profile_id, transaction_id, wallet_id, entry_type, asset, ordinal]`
  (compact JSON, ASCII). Set byte 6 to `(byte & 0x0f) | 0x80` and byte 8 to
  `(byte & 0x3f) | 0x80`, then emit the lowercase, hyphenated UUID string.
- **Ordinal.** The ordinal counts earlier entries of the same build with the
  same four values, in the engine's entry order, starting at zero.
- **Shape.** Ids keep today's 36-character shape. Nothing parses them as
  UUIDs.

A build reads the profile's rows and compares them with the new rows on
the explicit `ENTRY_COLUMNS` / `POSTING_COLUMNS` in
`kassiber/core/journal_storage.py`, excluding `created_at`. Entry REAL columns
`fiat_value`, `unit_cost`, `cost_basis`, `proceeds` and `gain_loss` compare the
sign of zero as well, because Python treats `0.0 == -0.0` as equal.

- Rows the build no longer produces are deleted.
- A tie group is every row with the same `occurred_at`. If any of its rows
  is new or changed, the whole group is written: its old rows are deleted,
  and its rows are inserted in engine order with the build's `created_at`.
- Every other row is left untouched and keeps its `created_at`.

**Why tie groups.** Readers order by `occurred_at, created_at, id`.

- Today all rows share one `created_at`, so the effective order is
  `occurred_at, id`.
- Rewriting a whole tie group keeps one `created_at` per `occurred_at`. The
  order therefore stays `occurred_at, id` regardless of build history, and
  equals what a build from scratch would give. No reader query changes.
- A tie group whose kept rows disagree on `created_at` (rows written outside
  Kassiber) is rewritten.

**Postings.** `journal_quantity_postings` already has deterministic
`posting_id`s and holds about 30% of the journal bytes written. It gets the
same changed-row write. Nothing reads its `created_at` or orders by it, so it
needs no tie rule. The other journal tables are small and keep full
replacement and their existing identity rules: quarantines, tax summary,
holdings, quantity issues and balances, custody decisions, relations,
reconciliations and gap inputs. Tax summaries and holdings keep uuid4 ids;
canonical custody decisions, issues and relations already have deterministic ids.

**Migration.** No schema change. The first build after the upgrade matches
none of the old uuid4 ids and rewrites every row, as every build does
today. Later builds write only changes. Rekeying at open would be a full
rewrite at launch for no gain, so there is no open-time step and no audit
row.

**An older binary with a newer book:**

- Older Python deletes and reinserts every row with uuid4 ids and one
  `created_at`, which keeps the tie-group property. The next current build
  rewrites everything once.
- Older readers in either runtime return rows in the same order.

**Visible changes:**

1. **Entry ids are stable** across rebuilds and change from random uuid4 to
   UUIDv8. They appear in:
   - `journals list` and `journals events list/get`;
   - `reports journal-entries`;
   - `reports capital-gains` (`explanation_reference.entry_id`);
   - `ui.journals.events.list`;
   - the transfer review context and review worklist;
   - the audit package and audit evidence summary;
   - the year-handoff workbook;
   - the derived pair id `journal-transfer:<out>:<in>`.
2. **Ties have a fixed order.** Entries with the same `occurred_at` stay in
   id order instead of a random order per rebuild. This affects:
   - row order in the journal, capital-gains, Austrian, year-handoff and
     verification outputs;
   - cutoffs at the desktop capital-gains limit (200), the journals snapshot
     limit (12) and event-list pages;
   - first-match journal-transfer pairing;
   - order-sensitive float sums in desktop cost-basis views.

   The Austrian CSV bundle's `content_sha256` becomes reproducible.
3. **`created_at` is the build that last wrote the row.** Before, it was
   the latest build. This shows in `journals events list/get`,
   `ui.journals.events.list` (`createdAt`) and list cursors.
   `ui.audit.changes_since_last_answer` changes with it:
   - `counts_since.journal_entries` counts rewritten entries only;
   - `latest.journal_entries` is the last time an entry changed;
   - unchanged entries no longer set `changed`; other rewritten tables,
     including quarantines, still contribute their own counts;
   - `latest.journals_processed_at` still reports the build.
4. Explanation references still expire with every rebuild, because they bind
   `processed_at`. When the build clock is no later than the persisted profile
   marker, the new marker is the previous instant plus one microsecond, emitted
   in UTC with six fractional digits. Otherwise it is the supplied build clock.
   If either timestamp is absent or unparsable, the supplied text is retained.
   This preserves expiry even for same-second rebuilds and clock rollback.
   Row `created_at` and filed-impact `rebuilt_at` still use the supplied clock.
   Quarantine freshness compares these markers as parsed instants, so an earlier
   error in the same second does not outlive a successful rebuild.
   A list cursor now survives a rebuild that leaves its rows unchanged.

Entry values, report totals, freshness and filed-report
impact resolution are unchanged.

**Risks:**

- **Hashes.** No hash covers journal rows except exported file bytes (the
  year-handoff workbook and Austrian CSV bundle), which already changed with
  every rebuild.
- **Collisions.** The `id` primary key is global, not per profile. The hash
  covers `profile_id`. A collision fails the `INSERT`, and the build rolls
  back with its freshness markers stale.
- **Privacy.** An id derives only from ids that the same outputs already
  carry. Stable ids link two exports of one book no more than their
  transaction ids already do. AI projections keep their shapes and
  redaction.
- **Atomicity.** The write stays inside `journals_process`'s savepoint and
  transaction. The desktop daemon, the freshness journal job and review
  apply all call the same function.
- **Unordered reads.** Readers without `ORDER BY` are counts, sets and
  per-transaction sums. The desktop's yearly realized-gain SUM now explicitly
  orders its inputs by `occurred_at, created_at, id`, matching the chronological
  index scan it previously used without relying on the query planner.
  Physical row order now follows write history
  between tie groups. Within a group it follows engine order, as a build
  from scratch would.

**R4-12 mirror.**

- The same id derivation, with ordinals in engine order.
- Entry construction consumes no uuid4 values. Other generated identities
  retain their existing call order.
- The same read, compare and tie-group rule.
- Deletes before inserts, with inserts in engine order.
- The same signed-zero REAL comparison.
- The posting changed-row write.
- Reject duplicate input ids even when they match retained rows. Posting ids
  are keyed by `(profile_id, posting_id)`; entry ids have the global primary key.
- Read the previous processing marker from the database, not the caller's
  potentially stale profile, and apply the microsecond successor rule above.

Both runtimes then leave identical database state from one starting copy
and clock. R4-12's dump comparison needs no id mapping for these two tables.

### 3. SQLCipher write mode

**WAL is not available.**

- Python's bundled `sqlcipher3` 0.6.2 embeds SQLite 3.51.1, which lacks the
  [WAL-reset fix](https://www.sqlite.org/wal.html#walresetbug).
  `_journal_settings_for_sqlite_version` already picks WAL for any
  connection whose SQLite has the fix, SQLCipher included.
- Rust's bundled SQLCipher 4.14.0 is SQLite 3.51.3, which has the fix.
- Every runtime that opens a book must have the fix, so Rust keeps
  SQLCipher on DELETE/FULL while Python can still open books.

**What does work.** SQLCipher's write cost is mostly the page cache spilling
mid-transaction. With Kassiber's 16 MB cache, each spill syncs the rollback
journal and encrypts pages that are written again later.

- A 128 MiB page cache for the journal transaction brings 2.28 s down to
  1.48 s.
- 64 MiB gives 1.65 s; 256 MiB gives the same as 128 MiB.
- WAL/NORMAL alone gives 1.87 s.
- TRUNCATE and PERSIST journaling gain nothing.
- Disabling spills with `cache_spill=OFF` is slower (3.0 s).
- Plaintext gains too: 0.87 → 0.74 s.

**On disk.** Nothing changes, and DELETE/FULL durability stays.
`process_journals` and review apply raise the connection's `cache_size` to
128 MiB (`-131072`) for their transaction. They restore the previous value
after commit or rollback, so the daemon's long-lived connection returns to
its prior cache size (normally 16,000 KiB). With review apply's `commit=False`,
the cache is restored on return while the caller still owns the pending
transaction and its commit or rollback. Nested scopes restore their own exact
signed `cache_size` value, including a positive page count. Explicit
`cache_spill` settings are untouched. Past 128 MiB of dirty pages, SQLite
spills as it does today under its existing spill setting.

**Migration.** None, because nothing is stored. Older binaries are
unaffected.

**Risks:**

- **Memory.** The prototype's peak memory rises by about 50 MB at generic
  20,000. The page-cache target is 128 MiB; whole-process memory can exceed it.
- **Decrypted pages in memory.** More decrypted pages stay in RAM during the
  build. The build already holds the book's rows in RAM, and nothing reaches
  disk or logs.

**R4-12 mirror.** The same scoped cache size around the rebuild
transaction.

**Optional owner decision.**

- WAL for SQLCipher books needs a `sqlcipher3` release whose SQLite has the
  fix. 0.6.3 and 0.6.4 exist; whether they embed SQLite 3.51.3 or later is
  unverified. Each platform wheel would need checking, and Rust's
  coexistence rule would need changing with it.
- WAL would add about 0.1–0.3 s on a full 20,000 rewrite on top of the
  cache. Its real benefit is that readers are not blocked during writes.
- Recommendation: not for speed now. Decide it as a concurrency question
  with the next dependency update.

### Compact-storage implementation measurements

These are real Python `journals process` wall times, including a fresh
interpreter, imports, book open, commit and close, comparing `ff7050c0` with
`6a94f676`. Every sample uses a disposable copy of a primed book. The
1-minute load exceeded 8 throughout, so before/after runs alternate AB/BA
and the table uses the best of five for each version. These measurements
do not include deterministic ids or the larger rebuild cache.

| Book | Plaintext, before → after (s) | SQLCipher, before → after (s) |
| --- | ---: | ---: |
| generic 2k | 2.028 → 2.063 | 2.686 → 2.623 |
| generic 20k | 13.779 → 13.802 | 17.398 → 16.388 |
| multi-wallet 2k | 2.550 → 2.589 | 3.340 → 3.244 |
| multi-wallet 20k | 24.391 → 23.395 | 27.416 → 26.959 |

Load ranged from 10.27 to 58.86. Generic 20k SQLCipher improves 5.8%;
plaintext generic timings are effectively unchanged. The compact format
reduces retained bytes, but custody and rendering still dominate the full
command, and these busy-host timings do not establish a small speedup.
Raw samples and runtime versions retain
each run's time and starting/ending load.

The report-dump harness freezes clocks and generated artifact ids, compares
every report command (including typed refusals), hashes PDF/CSV bytes and
XLSX/ZIP members, and captures audit evidence, audit packages, seven redacted
AI projections and every retained calculation's legacy bytes. Parity copies
use `ANALYZE journal_entries` after the first three completed cases: without
statistics, the full-population SQLCipher audit scan chooses the profile
index for every transaction instead of the existing transaction index.
Both versions receive identical statistics; the timed books use none of
this preparation.

The multi-wallet 20k dump exposed a pre-existing audit reader error:
`journal_quarantines` stores `detail_json`, but the package reader requested
`details` and crashed whenever a transaction was quarantined. The reader now
copies `detail_json` into the existing `details` output field and preserves
the blocking warning. Its regression fails before the correction. Report
parity applies that same one-line correction to the pre-compact baseline;
the original baseline cannot produce these quarantined audit packages.

All 45 captured sections match byte for byte in all ten comparisons:
generic and multi-wallet 2k/20k, and barrier 2k, each plaintext and
SQLCipher. Every retained calculation also reproduces its legacy bytes.
The dumps compare typed refusals for unavailable Lightning/cross-check/PSBT
inputs, Austrian reports on generic profiles, and blocked source-of-funds
exports. Those cases do not establish successful export coverage; the
report and accounting shards cover their supported success paths.

The first report-shard run also exposed an existing test-harness assumption:
with scratch under the worktree, pytest records repository-relative module
names. The tax-shadow harness now checks the exact failure count and probe
filename for either path rather than requiring a leading slash. Its injected
difference and builder-refusal assertions keep their failure semantics.

The custody census also exposed the cost-basis-policy fixture's intentional
Python-only `wallet` policy: Rust correctly retains the shipped `global`
allowlist. That fixture explicitly skips fusion only while its exact patched
policy builder is installed. All six preceding custody stages remain compared;
regressions prove that other-stage differences still fail and that the exception
does not survive its context. No product policy or comparison for real books
changes.

The required accounting, report, CLI/daemon and security shards pass. The
accounting census records 364 tax calls and 259 custody snapshots; the
custody-shadow lane compares 253 snapshots with the frozen Python flow. All
three Rust store manifests match, the offline Rust workspace passes, and the
fresh Rust read/interoperability/migration matrix passes 381 tests (2 skipped).
The authoritative logs and test summaries are linked from the results JSON.
Older-binary coexistence builds must use a separate Cargo target: reusing a
target here left a stale store artifact until `cargo clean -p kassiber-store`
and a fresh build. The recorded Rust gates use that fresh current store.
The pinned error/environment indexes are refreshed for the compact change's
source-line shifts; their error policies and environment reads are unchanged.

### Changed-row implementation measurements

These compare the compact baseline at `3c6c324c` (the `6a94f676` writer plus
the audit reader correction) with deterministic ids and changed-row writes,
before the cache change. Each real Python CLI command
rebuilds an unchanged, primed disposable book; before/after runs alternate
AB/BA, with five samples per version. These are end-to-end times, including
imports, book open, commit and close.

| Book | Plaintext, before → after (s) | SQLCipher, before → after (s) |
| --- | ---: | ---: |
| generic 2k | 1.914 → 1.844 | 2.224 → 2.113 |
| generic 20k | 12.682 → 12.133 | 14.220 → 12.367 |
| multi-wallet 2k | 2.296 → 2.299 | 2.672 → 2.552 |
| multi-wallet 20k | 21.762 → 21.035 | 24.072 → 21.820 |

Load ranged from 1.65 to 3.01. At 20k, SQLCipher improves 13.0% on generic
and 9.4% on multi-wallet; plaintext improves 4.3% and 3.3%. Multi-wallet 2k
plaintext is unchanged. Compare each pair within this table: host load is
lower than in the earlier compact-storage measurements, so joining the two
tables' endpoints would overstate the combined gain.

The report comparison canonicalizes the approved id and tie-order changes
in the baseline entry reader before comparing output. It also removes that
reader's per-entry uuid4 consumption so unrelated generated artifact ids
remain aligned. It freezes clocks and artifact identities; it does not claim
raw old uuid4 exports equal new UUIDv8 exports. No output fields or numeric
values are discarded from the comparison. No-op audit/cursor behavior and
same-second explanation expiry have separate regressions.

All ten book/encryption cases match across 54 captured sections each: every
report command, generated file and workbook/archive member, audit evidence,
audit package, retained calculations, 15 successful redacted AI projections and
a matching dashboard validation refusal. A separate ten-case dashboard success
comparison between `3c6c324c` and `5ad2ab0c`, with `period: all` and the same
approved baseline id canonicalization, also matches in full. As in the
compact comparison, commands missing required bench inputs match their errors;
their success paths depend on the accounting/report shards. Full compressed
dumps remain under `target/r4-storage/results/dumps/changed-*`; their hashes,
raw timing samples and coexistence checks are in
the recorded results.

Plaintext and SQLCipher coexistence checks show an older compact Python writer
replacing UUIDv8 with uuid4, current Python restoring the same UUIDv8 ids, and
the next current no-op preserving every rowid and stored column. The current
Rust store reads both books. The compact section's pre-column Rust refusal
and old Python legacy-calculation rules still apply.

The final reader review found that the yearly realized-gain aggregate lacked
an explicit input order. A scan-direction regression changed its floating sum
from 0 to 1e-16, even with SQLite's compensated SUM. Its ordered subquery now
keeps journal order on either driver, independently of physical write history.
The bench's 2024 dates do not exercise the real-clock YTD window; this dedicated
regression uses future dates to cover that gap.

The fractional-marker reader review also found a text comparison in quarantine
freshness. A same-second error incorrectly survived a successful rebuild whose
marker had a fractional successor. Its regression fails before the correction;
parsed timestamp comparisons now clear the earlier error and keep later errors.

The required report, CLI/daemon and security shards pass. Core accounting with
the tax/custody census passes 1,951 tests (393 tax calls, 288 custody snapshots;
285 fusion comparisons and three narrow fixture exceptions); the custody-shadow
lane passes 601 tests with 393 tax calls. All three Rust store manifests, the
offline Rust workspace and the Rust read/interoperability/migration matrix pass.
The latter passes 381 tests (two skipped). The final storage, explanation and
quarantine probes pass 85 tests, including encrypted backup/restore.
The final inventory and source-contract refresh and preflight pass. The storage
and explanation probes also cover repeated input ids, cross-profile posting
keys, global entry-id collision rollback, no-op cursors, retained audit times,
and same-second or backward-clock explanation expiry. Two old test expectations
were updated for UUIDv8 and retained posting creation times; frozen accounting
vectors are unchanged.

### Scoped-cache implementation measurements

These compare `5ad2ab0c` with the 128 MiB cache. Every real Python CLI command
starts from a disposable primed copy with `journal_entries` and
`journal_quantity_postings` emptied before timing, exercising a full rewrite of
both large tables. Counts are checked after timing to prove both were rebuilt.
The command time includes imports, book open, processing, commit and close;
the preparation and verification are outside it. Before/after runs alternate
AB/BA, with five samples per version.

| Book | Plaintext, before → after (s) | SQLCipher, before → after (s) |
| --- | ---: | ---: |
| generic 2k | 1.862 → 1.907 | 2.110 → 2.149 |
| generic 20k | 13.165 → 12.786 | 13.940 → 13.411 |
| multi-wallet 2k | 2.352 → 2.345 | 2.664 → 2.707 |
| multi-wallet 20k | 22.496 → 22.478 | 23.810 → 22.685 |

Load ranged from 3.96 to 7.58. At 20k, SQLCipher improves 3.8% on generic
and 4.7% on multi-wallet; plaintext improves 2.9% and 0.1%. The 2k results
range from 2.4% slower to 0.3% faster and establish no consistent gain.
These full rewrites differ from the changed-row section's unchanged rebuilds;
do not combine their endpoints or infer a no-op cache speedup.

All ten book/encryption cases match across the same 54 captured sections,
without id mapping: report envelopes, file/member hashes, audit evidence and
packages, retained calculations and 16 successful redacted AI projections.
The dashboard capture uses `period: all` so it exercises its full success path.
Report commands without the required bench inputs still compare their typed
refusals; supported success cases also depend on the accounting/report shards.

The cache regression fails on both drivers before the change. Focused storage
and review verification passes 71 tests, including success/failure restoration,
positive prior page counts, nested exceptions, explicit spill settings and a
caller-owned review transaction with `commit=False`. Backup/restore remains
covered by the storage regressions. This connection-only change needs no new
schema manifest or migration; the compact column's Rust mirror remains current.

The final required report, daemon/CLI and security shards pass. Core accounting
under the tax/custody census passes 1,963 tests with 421 tax calls and 316
finalized custody comparisons; fusion records 313 comparisons and the same three
narrow fixture exceptions. Custody shadow with tax census passes 613 tests and
421 tax calls. All three Rust store manifests match.

The offline Rust workspace and fresh Rust read/interoperability/migration matrix
pass; the latter passes 381 tests (two skipped). The final classifier and
source-contract refresh and preflight pass (57 tests and 57 subtests). Manual
second review found no outstanding correctness or consistency issues.


## Decisions for the owner

The owner answered decisions 1 to 4 on 2026-10-30. Decisions 5 and 6 remain open.

1. **Early production path.** R4 makes the Rust binary authoritative.
   Desktop users get the speed-up at the R7 cutover, because the Python
   sidecar stays in the product until then. An optional PyO3 path would have
   the Python product call the typed pipeline for compute only (SQL and
   persistence stay Python). Measured cost of its JSON boundary: 0.33 s to
   serialize the snapshot plus Rust's parse at 20,000, with a compact result
   instead of today's 270 MB stage dump. Recommendation: no; spend the
   effort on R4-9 to R4-15.

   **Decided: no early PyO3 production path.** The recommendation stands.
2. **Targets.** Accept 10× for custody and ≥ 4× end to end (the budget
   above) as R4's performance acceptance? Separately, should R4 include a tax
   engine performance slice (0.55–1.5 s of compute at 20,000)? It must keep
   engine output byte-identical; whether its cost can be halved is unmeasured.

   **Decided: both targets are accepted** as R4's performance acceptance:
   custody at least 10× Python, end to end at least 4×. A tax-engine speed
   pass is part of R4. It runs separately, on `t3code/ledger-perf`.

   The separately authorized speed pass meets its ≥ 2× multi-wallet 20k
   target: 1.737 → 0.439 s for text-in/text-out `ledger`, with byte-identical
   output. The original engine budget is not a complete-pipeline measurement.
3. **Automatic pairing.** The desktop button and the freshness journal job
   run the transfer matcher four times (3.4–4.7 s at 20,000). Port it in R4
   (R4-14), or route only plain `journals process` to Rust in R4 and switch
   `--auto-pair`, `ui.journals.process` and the freshness job when the R5
   transfers slice lands? Recommendation: port it in R4, because the daemon
   journal path is R4's scope and otherwise stays Python.

   **Decided: port automatic pairing in R4 now** (R4-14), in a separate
   worker run.
4. **Beyond parity.** Going past about 4–5× end to end needs persisted-shape
   changes, each a semantic change outside this programme's parity rule:
   - `calculation_json` is 72% of `journal_entries` bytes;
   - deterministic entry ids would allow writing only changed rows (today
     `uuid4` ids and `created_at` change every row on every rebuild);
   - SQLCipher books would need WAL once the SQLCipher SQLite has the
     WAL-reset fix.

   Defer them all until after R7? Decided 2026-10-30: do them now. See
   [storage-format speedups](#storage-format-speedups).
5. **Book open cost.** The R5 binary spends 0.10 s opening a 2,000-transaction
   book. That is a fifth of the 2,000-transaction budget. Should R4 trim the
   open path, or should R3/R5 own it? **Open.**
6. **Allocator.** Allow measuring an alternative allocator, which is a
   dependency change, if contention is still visible after R4-7? **Open.**

Separately, the owner approved one new index, on
`transactions(profile_id, occurred_at, created_at, id, wallet_id)` (the
transaction page index). No other index was approved.
