# R4 journal pipeline design

**Status:** R4-0 round 4 integrated, 2026-10-31; the R4-1 bench harness is
committed and its accepted baseline remains open. The R4-11 renderer has a
[partial checkpoint](r4-journal-render-checkpoint.md); production acceptance
remains open. [Storage-format speedups](#storage-format-speedups) run Python
first; compact calculations are implemented with the Rust schema mirror.
The [R7 integration audit](rust-backend-r7-integration.md) records the linear
restack and its verification. The separate [tax-engine speed pass](tax-engine.md#ledger-performance-checkpoint-r4-2026-10-30)
measures 2.03–3.96× faster `ledger` calls with byte-identical responses; its
timings supersede the engine line in this design's original measurements.
This document plans stage R4 of
[plan 21](../plan/21-rust-backend.md): `journals process` end to end in the Rust
binary, byte-identical to Python and much faster. It builds on the
[custody engine](custody-engine.md) (R2, including the R2g fusion boundary and
the performance checkpoint) and on the storage and service-slice work of the
R3/R5 stack.

Two measurements shape the whole design:

- Python spends most of its time in custody and in the Python halves of the
  ledger path. Rust custody behind the JSON binding is barely faster than
  Python on generic books, so the in-process path needs typed data, not
  another round of tuning on `serde_json::Value`.
- SQLite writes have a floor of about 1.25–1.7 s for a 20,000-transaction
  plaintext book and about 3 s for a SQLCipher one, whatever language issues
  them. Under byte parity, end-to-end `journals process` therefore cannot
  reach 10× at 20,000 transactions. The compute pipeline can, and this design
  sets 10× there.

## Measured phase profile

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
- `tax-engine/core` gains a typed entry point, `ledger_typed(&LedgerRequest)
  -> Result<LedgerResponse, StagedError>`, beside today's `ledger(&str)`. The
  text entry stays for Python and fixtures.
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
| Load | `BookSnapshot { profile, book_network, wallets: Vec<WalletRef>, transactions: Vec<TxRow>, rate_rows: Vec<RateRow>, owned_index: Option<OwnedIndex>, components: Vec<AuthoredComponent>, anchors, anchor_wallets, revision_headers, receipt_sources, channel_records, loan_legs, evidence_snapshots, migration_issues, gap_reviews, pair_dismissals, gap_worklist }` | `load_custody_snapshot` dicts; `Snapshot` with `Record = Map<String, Value>` |
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
catalog (2,246 recipes), and reports kill set equals catalog. The catalog may
not shrink without a reviewed equivalence note. Each step also records
before and after timings from the bench books, back to back.

"Measured cost replaced" is what the code a step rewrites costs today, at
generic / multi-wallet 20,000 transactions. It bounds the step's gain; it is
not a prediction. Each step's target is an acceptance criterion. A step that
misses it stops for re-planning before the next step starts.

| Step | Work | Measured cost replaced (20k) | Step target |
| --- | --- | --- | --- |
| R4-0 | Join the stacks: restack `r2g-fusion-6` (7 commits) on `r2-custody-perf`, then onto the R5 tip, whose custody stops at the R2c channel-lifecycle checkpoint. Resolve `lib.rs`/codec conflicts. | — | All gates green; hashes unchanged |
| R4-1 | Bench harness in the Rust workspace: an end-to-end `journals-bench` example with spans, committed bench-book generators, the Python profiler, a regression record format | — | Baseline recorded |
| R4-2 | Typed `TxRow`, `WalletRef`, `RateRow`, worklist rows and `SqlValue`; drop the shared `Value` row view; admission over fields | parse 0.70 / 0.66; admission 0.27 / 0.36 | parse + admission ≤ 0.25 s at generic 20k |
| R4-3 | Typed components and evidence | 0.33 / 1.15 | ≤ 1/3 of today |
| R4-4 | Typed interpretation (inputs, claims, quarantines, audits) | 0.46 / 1.42 | ≤ 1/3 |
| R4-5 | Typed gap search | 0.36 / 0.38 | ≤ 1/2 |
| R4-6 | Typed quantity state; remove the Python-model conversion | 1.58 / 1.82 | ≤ 1/4 |
| R4-7 | Typed finalized projection; custody in-process total checked | 1.05 / 1.16 | custody ≤ 0.51 s generic, ≤ 1.21 s multi-wallet (10× Python) |
| R4-8 | Typed fusion to `LedgerRequest`, `ledger_typed` in the engine, text only for refusal fidelity and comparison | builder 1.31 / 1.42, wire 0.19, engine parse and serialize about 0.08 | request ≤ 0.10 s; engine no slower |
| R4-9 | Native SQLite loader to `BookSnapshot`, including book-network facts, the owned index (descriptor derivation from the R5 descriptor slice) and receipt sources; v1 byte gate against Python | Python load 1.57 / 1.89 (floor 0.11) | ≤ 0.30 s |
| R4-10 | Pre-build repairs and auto-pricing in Rust; database-dump parity | Python 0.03 | ≤ 0.03 s |
| R4-11 | Rendering and joins: entries, quarantine enrichment with ownership candidates, holdings, balance comparison, ownership review; [partial implementation](r4-journal-render-checkpoint.md), acceptance remains open | Python 1.38 / 1.71 | ≤ 0.15 s |
| R4-12 | Persistence: typed rows to prepared statements, freshness markers, filed-report impact resolution, deferred checkpoint; database-dump parity | Python 2.25 / 2.55 (deferred-checkpoint floor 1.25 / 1.45) | ≤ floor + 0.15 s |
| R4-13 | CLI `journals process` and daemon `ui.journals.process` without automatic pairing in the binary; progress phases; hybrid routing | — | End-to-end targets below |
| R4-14 | Automatic pairing (transfer suggestions, rules, bulk exact) for `--auto-pair` and the desktop path (decision 3: ported in R4) | Python 3.44 / 4.74 | 10× Python |
| R4-15 | Freshness `journal_refresh` job and the daemon's single execution slot in Rust | — | Parity with the daemon smoke sessions |
| R4-16 | Authority switch (below) | — | All gates and budgets |

R4-2 to R4-8 change no product behavior. Python still owns production until
R4-16. Order follows the data flow, because every stage reads the rows. A
stage whose neighbor is not typed yet converts at its boundary. That costs
time until the neighbor lands, so intermediate steps can be slower on one
book. The step record says so, and the next step must recover it.

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

Accepted baseline records will be committed as `r4-bench-records.jsonl`;
no accepted record is checked in yet. Evidence of each run stays with the
step's local verification.

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

- In the Rust binary, `journals process` stops answering `not_implemented`,
  and `commands describe` lists it. There is no Python fallback: a Rust
  refusal is the command's answer. The daemon kinds and the freshness journal
  job switch with it (R4-13 to R4-15).
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
  (compact JSON, ASCII).
- **Ordinal.** The ordinal counts earlier entries of the same build with the
  same four values, in the engine's entry order.
- **Shape.** Ids keep today's 36-character shape. Nothing parses them as
  UUIDs.

A build reads the profile's rows and compares them with the new rows on
their stored values. REAL columns compare the sign of zero as well, because
Python treats `0.0 == -0.0` as equal.

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
replacement and uuid4 ids: quarantines, tax summary, holdings, quantity
issues and balances, custody decisions, relations, reconciliations and gap
inputs.

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
   - a rebuild that changes no entry no longer sets `changed` by itself;
   - `latest.journals_processed_at` still reports the build.
4. Explanation references still expire with every rebuild, because they bind
   `processed_at`. A list cursor now survives a rebuild that leaves its rows
   unchanged.

Values, every other column, report totals, freshness and filed-report
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
  per-transaction sums. Physical row order now follows write history
  between tie groups. Within a group it follows engine order, as a build
  from scratch would.

**R4-12 mirror.**

- The same id derivation, with ordinals in engine order.
- The same read, compare and tie-group rule.
- Deletes before inserts, with inserts in engine order.
- The same signed-zero REAL comparison.
- The posting changed-row write.

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
16 MB. Past 128 MiB of dirty pages, SQLite spills as it does today.

**Migration.** None, because nothing is stored. Older binaries are
unaffected.

**Risks:**

- **Memory.** Peak memory rises by about 50 MB at generic 20,000, at most
  128 MiB.
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

## Decisions

The owner answered decisions 1 to 4 on 2026-10-30. Decisions 5 and 6 remain
open.

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
